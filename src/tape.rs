// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 (datasette.c, tap.c, t64.c, lib.c),
// Copyright (C) Andreas Boose, Andreas Matthies, Marco van den Heuvel,
// David Hansel, Ettore Perazzoli, Compyx.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! Tape recorder (Datasette 1530) and tape images, as in VICE 3.10
//! (datasette.c, tap.c, t64.c).
//!
//! Connections to the C64:
//! - READ: every tape pulse is a falling edge on the CIA1 FLAG line (ICR
//!   bit 4);
//! - WRITE: bit 3 of $01; while recording every rising edge ends a pulse,
//!   as long as the time since the previous edge;
//! - SENSE: bit 4 of $01, 0 when a button (PLAY, RECORD, FF, REW) is down;
//! - MOTOR: bit 5 of $01, the motor runs while the line is low. The motor
//!   takes 32000 cycles to start and as many to stop (1000 for fast
//!   winding). FF and REW too move the tape only with the motor on: the
//!   KERNAL turns it on when it sees a button pressed.
//!
//! A TAP stores the gap between one pulse and the next: one byte ×8
//! cycles, or 0 followed by the exact duration in 3 bytes (version 1; in
//! version 0 the zero means 2500 cycles, as in mtap). The tape speed wobbles
//! as in VICE (by default ±0.5% at "3 Hz", see `wobble`); comparisons with
//! VICE are also made with `-dstapewobbleamp 0` and `set_wobble(0, 0)`.
//!
//! Off by default as in VICE: the azimuth error (misaligned head: every
//! pulse is off by a few random cycles, see `set_azimuth_error`) and the
//! sound of the tape running in PLAY (one square wave per pulse, mixed
//! into the SID audio with `mix_sound`).

/// Motor delay at start and stop (PLAY and RECORD).
const MOTOR_DELAY: u64 = 32_000;
/// Delay at the start of fast winding (FF and REW).
const MOTOR_DELAY_FAST: u64 = 1000;
/// Maximum time between two tape checks: longer pulses are split.
const MAX_GAP: u64 = 100_000;
/// Duration of a zero in a version 0 TAP (as in mtap).
const ZERO_GAP_DELAY: u64 = 2500;

// Datasette mechanics (datasette.h): the counter is
// c = g·(√(v·t/(d·π) + r²/d²) − r/d), with t the seconds of tape from the start.
const PI: f64 = 3.1415926535;
/// Tape thickness (m).
const DS_D: f64 = 1.27e-5;
/// Radius of the empty reel (m).
const DS_R: f64 = 1.07e-2;
/// Tape speed in PLAY (m/s).
const DS_V_PLAY: f64 = 4.76e-2;
/// Counter ratio.
const DS_G: f64 = 0.525;
/// Counter revolutions per second in FF and REW.
const DS_RPS_FAST: f64 = 4.00;

/// Default tape speed wobble, in VICE units: amplitude in 1/100000
/// (500 = ±0.5%), frequency in 1/1000 Hz (3000 = 3 Hz).
pub const WOBBLE_AMPLITUDE_DEFAULT: u32 = 500;
pub const WOBBLE_FREQUENCY_DEFAULT: u32 = 3000;
pub const WOBBLE_AMPLITUDE_ONE: u32 = 100_000;
pub const WOBBLE_FREQUENCY_ONE: u32 = 1000;
/// Azimuth error in thousandths of a cycle (VICE's -dstapeerror), at most
/// ±10 cycles.
pub const AZIMUTH_ERROR_ONE: u32 = 1000;
pub const AZIMUTH_ERROR_MAX: u32 = 10 * AZIMUTH_ERROR_ONE;
/// Tape sound volume (VICE's -dssoundvolume): amplitude of the square
/// wave on the SIDs' 16-bit scale.
pub const SOUND_VOLUME_DEFAULT: u16 = 1024;
pub const SOUND_VOLUME_MAX: u16 = 4096;

/// TAP image: header and pulse data.
#[derive(Default, Clone)]
pub struct Tap {
    pub version: u8,
    /// System and video standard from the header (0 = C64, 0 = PAL).
    pub system: u8,
    pub video: u8,
    pub data: Vec<u8>,
    /// The tape has been recorded on: it must be written back to its file.
    pub changed: bool,
}

impl Tap {
    pub fn from_bytes(bytes: &[u8]) -> Result<Tap, String> {
        if bytes.len() < 20 || &bytes[..12] != b"C64-TAPE-RAW" {
            return Err("not a C64 TAP image (C64-TAPE-RAW missing)".into());
        }
        let version = bytes[12];
        if version > 2 {
            return Err(format!("unsupported TAP version {version}"));
        }
        let size = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let end = (20 + size).min(bytes.len());
        Ok(Tap { version, system: bytes[13], video: bytes[14], data: bytes[20..end].to_vec(), changed: false })
    }

    /// Blank tape as VICE creates it (tap_create): version 1, four zero
    /// bytes (a "long" pulse of zero duration).
    pub fn blank() -> Tap {
        Tap { version: 1, system: 0, video: 0, data: vec![0; 4], changed: false }
    }

    /// TAP file: header with the data length, then the data.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = b"C64-TAPE-RAW".to_vec();
        out.extend_from_slice(&[self.version, self.system, self.video, 0]);
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

/// Extracts the first program from a T64 image (a file container): load
/// address and data, like a PRG.
pub fn t64_first_prg(bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    if bytes.len() < 64 || !bytes.starts_with(b"C64") {
        return Err("not a T64 image".into());
    }
    let max = u16::from_le_bytes([bytes[0x22], bytes[0x23]]).max(1) as usize;
    let entries: Vec<&[u8]> = (0..max)
        .map(|i| 0x40 + 32 * i)
        .take_while(|&o| o + 32 <= bytes.len())
        .map(|o| &bytes[o..o + 32])
        .collect();
    // Used entries (type 1: normal file), sorted by offset in the file
    let mut files: Vec<(usize, u16, u16, String)> = entries
        .iter()
        .filter(|e| e[0] == 1)
        .map(|e| {
            let start = u16::from_le_bytes([e[2], e[3]]);
            let end = u16::from_le_bytes([e[4], e[5]]);
            let offset = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let name: String = e[16..32].iter().map(|&c| c as char).collect::<String>()
                .trim_end_matches([' ', '\0', '\u{A0}']).to_string();
            (offset, start, end, name)
        })
        .collect();
    files.sort_by_key(|f| f.0);
    let Some(&(offset, start, end, ref name)) = files.first() else {
        return Err("the T64 image contains no files".into());
    };
    if offset >= bytes.len() {
        return Err("T64: file offset outside the image".into());
    }
    // The end address is often wrong in T64s (usually $C3C6): the length
    // is limited to the next file or to the end of the image
    let limit = files.get(1).map_or(bytes.len(), |f| f.0.min(bytes.len())) - offset;
    let len = if end > start { ((end - start) as usize).min(limit) } else { limit };
    let mut prg = start.to_le_bytes().to_vec();
    prg.extend_from_slice(&bytes[offset..offset + len]);
    Ok((name.clone(), prg))
}

/// Datasette buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Stop,
    Play,
    /// Fast forward.
    Forward,
    /// Rewind.
    Rewind,
    /// RECORD (with PLAY).
    Record,
}

impl Button {
    pub fn name(self) -> &'static str {
        match self {
            Button::Stop => "STOP",
            Button::Play => "PLAY",
            Button::Forward => "FF",
            Button::Rewind => "REW",
            Button::Record => "RECORD",
        }
    }
}

pub struct Datasette {
    pub image: Option<Tap>,
    /// Position in the TAP data.
    pos: usize,
    button: Button,
    /// The motor is running (also during the stop delay).
    motor: bool,
    /// CPU clock, Hz (PAL or NTSC): the speed of the counter, of fast
    /// winding and of the wobble, as VICE's datasette_cycles_per_second.
    clock_hz: f64,
    /// Cycle at which the motor stops (0: no stop in progress).
    motor_stop_clk: u64,
    /// MOTOR line last seen on the processor port.
    motor_line: bool,
    /// Next tape check (pulse or motor stop).
    alarm: Option<u64>,
    /// Part not yet elapsed of a pulse longer than MAX_GAP, or of a pulse
    /// interrupted by a change of direction.
    long_gap_pending: u64,
    /// Part of the current pulse already elapsed.
    long_gap_elapsed: u64,
    /// Direction of the last movement: 1 forward, -1 backward, 0 none.
    last_direction: i32,
    /// Tape position and length in units of 8 cycles, for the counter and
    /// the fast winding speed.
    cycle_counter: u64,
    cycle_counter_total: u64,
    /// Counter value reset with the Datasette's button.
    counter_offset: i32,
    /// Recording: cycle of the last edge written (0: none) and state of the
    /// WRITE line as seen by the Datasette (-1 at power-on).
    last_write_clk: u64,
    last_write_bit: i32,
    /// WRITE line on the processor port (None: never read).
    write_line: Option<bool>,
    /// Speed wobble, in VICE units (0: off).
    wobble_amplitude: u32,
    wobble_frequency: u32,
    /// Wobble phase and rounding remainder of the pulses.
    wobble_phase: f32,
    wobble_rest: f32,
    /// Azimuth error in thousandths of a cycle (0: off), remainder carried
    /// to the next pulse and random generator state.
    azimuth_error: u32,
    azimuth_rest: i64,
    rng: u64,
    /// Tape sound: volume (0: off), sign of the next half-wave, level
    /// changes still to be mixed (cycle, level), level and cycle from which
    /// mixing resumes.
    sound_volume: u16,
    sound_sign: i8,
    sound_edges: Vec<(u64, i16)>,
    sound_level: i16,
    sound_from: u64,
    /// Pulses read (for the load turbo).
    pub pulses: u64,
    /// Cycle since which the tape has been running without stopping (motor
    /// on and a button down).
    run_start: u64,
}

impl Datasette {
    pub fn new() -> Self {
        Self {
            image: None,
            pos: 0,
            button: Button::Stop,
            motor: false,
            clock_hz: crate::timing::Standard::Pal.clock_hz() as f64,
            motor_stop_clk: 0,
            motor_line: false,
            alarm: None,
            long_gap_pending: 0,
            long_gap_elapsed: 0,
            last_direction: 0,
            cycle_counter: 0,
            cycle_counter_total: 0,
            counter_offset: 0,
            last_write_clk: 0,
            last_write_bit: -1,
            write_line: None,
            wobble_amplitude: WOBBLE_AMPLITUDE_DEFAULT,
            wobble_frequency: WOBBLE_FREQUENCY_DEFAULT,
            wobble_phase: 0.0,
            wobble_rest: 0.0,
            azimuth_error: 0,
            azimuth_rest: 0,
            rng: PCG_MULTIPLIER.wrapping_add(PCG_INCREMENT),
            sound_volume: 0,
            sound_sign: 1,
            sound_edges: Vec::new(),
            sound_level: 0,
            sound_from: 0,
            pulses: 0,
            run_start: 0,
        }
    }

    /// Inserts a tape, rewound and with the Datasette stopped (VICE's
    /// datasette_set_tape_image). For the counter the tape length is
    /// measured by reading it all, and as in VICE the reading also advances
    /// the speed wobble.
    /// CPU clock (PAL or NTSC), Hz. TAP pulses are in cycles and are played
    /// as they are, as VICE does.
    pub fn set_clock(&mut self, clock_hz: u64) {
        self.clock_hz = clock_hz as f64;
    }

    pub fn insert(&mut self, tap: Tap) {
        self.image = Some(tap);
        self.reset();
        self.cycle_counter_total = 0;
        loop {
            let gap = self.read_gap(1);
            self.cycle_counter_total += gap / 8;
            if gap == 0 {
                break;
            }
        }
        self.pos = 0;
    }

    /// Removes the tape, after pressing STOP.
    pub fn eject(&mut self) -> Option<Tap> {
        self.stop();
        let tap = self.image.take();
        self.reset();
        tap
    }

    /// C64 reset (like VICE's DatasetteResetWithCPU): Datasette stopped,
    /// tape rewound and counter at zero.
    pub fn reset(&mut self) {
        if matches!(self.button, Button::Play | Button::Forward | Button::Rewind) {
            self.alarm = None;
        }
        self.stop();
        self.pos = 0;
        self.cycle_counter = 0;
        self.counter_offset = 0;
        self.long_gap_pending = 0;
        self.long_gap_elapsed = 0;
        self.last_direction = 0;
        self.motor_stop_clk = 0;
    }

    pub fn button(&self) -> Button {
        self.button
    }

    /// Position on the tape and length, in TAP bytes.
    pub fn position(&self) -> (usize, usize) {
        (self.pos, self.image.as_ref().map_or(0, |t| t.data.len()))
    }

    /// Datasette counter (000-999), computed as VICE does from the position
    /// on the tape.
    pub fn counter(&self) -> u32 {
        if self.image.is_none() {
            return (1000 - self.counter_offset) as u32 % 1000;
        }
        (1000 - self.counter_offset as i64 + self.counter_turns() as i64).rem_euclid(1000) as u32
    }

    /// Resets the counter to zero.
    pub fn reset_counter(&mut self) {
        self.counter_offset = if self.image.is_none() { 1000 } else { ((1000 + self.counter_turns() as i64) % 1000) as i32 };
    }

    fn counter_turns(&self) -> i32 {
        let c1 = DS_V_PLAY / DS_D / PI;
        let c2 = (DS_R * DS_R) / (DS_D * DS_D);
        let c3 = DS_R / DS_D;
        (DS_G * ((self.cycle_counter as f64 / (self.clock_hz / 8.0) * c1 + c2).sqrt() - c3)) as i32
    }

    pub fn motor(&self) -> bool {
        self.motor
    }

    /// The tape is running: a button down and the motor on.
    pub fn moving(&self) -> bool {
        self.image.is_some() && self.button != Button::Stop && self.motor
    }

    /// Cycle since which the tape has been running without interruption, if
    /// it is running.
    pub fn moving_since(&self) -> Option<u64> {
        self.moving().then_some(self.run_start)
    }

    /// SENSE line: a button is down.
    pub fn sense(&self) -> bool {
        self.button != Button::Stop
    }

    /// Tape speed wobble in VICE units (-dstapewobbleamp,
    /// -dstapewobblefreq): amplitude in 1/100000, frequency in 1/1000 Hz.
    /// With either of them at 0 it is off.
    pub fn set_wobble(&mut self, amplitude: u32, frequency: u32) {
        self.wobble_amplitude = amplitude.min(WOBBLE_AMPLITUDE_ONE / 2);
        self.wobble_frequency = frequency.min(WOBBLE_FREQUENCY_ONE * 50);
    }

    pub fn wobble(&self) -> (u32, u32) {
        (self.wobble_amplitude, self.wobble_frequency)
    }

    /// Azimuth error in thousandths of a cycle (0: off, at most 10000):
    /// every gap moves randomly by at most that much.
    pub fn set_azimuth_error(&mut self, error: u32) {
        self.azimuth_error = error.min(AZIMUTH_ERROR_MAX);
        self.azimuth_rest = 0;
    }

    pub fn azimuth_error(&self) -> u32 {
        self.azimuth_error
    }

    /// Sound of the tape in PLAY at the given volume (`None`: off).
    pub fn set_sound(&mut self, volume: Option<u16>) {
        self.sound_volume = volume.map_or(0, |v| v.min(SOUND_VOLUME_MAX));
        if self.sound_volume == 0 {
            self.sound_edges.clear();
            self.sound_level = 0;
        }
    }

    pub fn sound(&self) -> Option<u16> {
        (self.sound_volume > 0).then_some(self.sound_volume)
    }

    /// Mixes the tape sound up to cycle `until` into `buf` (samples
    /// interleaved over `channels` channels, all those produced by the SIDs
    /// since the last call). Each sample is the average of the square wave
    /// over its cycle interval. Without samples (audio off) the sound is
    /// discarded.
    pub fn mix_sound(&mut self, buf: &mut [i16], channels: usize, until: u64) {
        let from = std::mem::replace(&mut self.sound_from, until);
        if self.sound_edges.is_empty() && self.sound_level == 0 {
            return;
        }
        let n = buf.len() / channels.max(1);
        let mut level = self.sound_level as f64;
        let mut used = 0;
        if n > 0 && until > from {
            let step = (until - from) as f64 / n as f64;
            for k in 0..n {
                let mut t = from as f64 + k as f64 * step;
                let end = t + step;
                let mut sum = 0.0;
                while let Some(&(at, l)) = self.sound_edges.get(used) {
                    let at = at as f64;
                    if at >= end {
                        break;
                    }
                    if at > t {
                        sum += level * (at - t);
                        t = at;
                    }
                    level = l as f64;
                    used += 1;
                }
                sum += level * (end - t);
                let v = (sum / step).round() as i16;
                if v != 0 {
                    for c in &mut buf[k * channels..(k + 1) * channels] {
                        *c = c.saturating_add(v);
                    }
                }
            }
        } else {
            while let Some(&(at, l)) = self.sound_edges.get(used) {
                if at >= until {
                    break;
                }
                level = l as f64;
                used += 1;
            }
        }
        self.sound_edges.drain(..used);
        self.sound_level = level as i16;
    }

    /// Sound of a gap of `gap` cycles read in PLAY starting at cycle `start`
    /// (VICE's datasette-sound.c): one period of square wave, half positive
    /// and half negative; in half-wave TAPs (version 2) every gap is a
    /// half-wave. After the last gap the sound goes silent.
    fn add_sound(&mut self, start: u64, gap: u64) {
        let v = self.sound_volume as i16;
        let half_waves = self.image.as_ref().is_some_and(|t| t.version == 2);
        if let Some(last) = self.sound_edges.last() {
            if last.0 == start && last.1 == 0 {
                self.sound_edges.pop();
            }
        }
        if self.sound_sign == 1 && !half_waves {
            self.sound_edges.push((start, v));
            self.sound_edges.push((start + gap / 2, -v));
        } else {
            self.sound_edges.push((start, v * self.sound_sign as i16));
            self.sound_sign = -self.sound_sign;
        }
        self.sound_edges.push((start + gap, 0));
    }

    /// Presses a Datasette button at cycle `clk` (VICE's
    /// datasette_control_internal). RECORD cannot be pressed without a
    /// tape.
    pub fn press(&mut self, button: Button, clk: u64) {
        if button == Button::Record && self.image.is_none() {
            return;
        }
        if button != Button::Stop && self.button == Button::Stop {
            self.run_start = clk;
        }
        self.button = button;
        self.last_write_clk = 0;
        if matches!(button, Button::Forward | Button::Rewind) {
            self.alarm = Some(clk + MOTOR_DELAY_FAST);
        }
        if matches!(button, Button::Play | Button::Forward | Button::Rewind) && self.motor {
            self.start_motor(clk);
        }
    }

    pub fn play(&mut self, clk: u64) {
        self.press(Button::Play, clk);
    }

    pub fn stop(&mut self) {
        self.button = Button::Stop;
        self.last_write_clk = 0;
    }

    /// Rewinds the tape to the start instantly (without going through the
    /// REW button).
    pub fn rewind(&mut self) {
        self.pos = 0;
        self.cycle_counter = 0;
        self.long_gap_pending = 0;
        self.long_gap_elapsed = 0;
        self.last_direction = 0;
    }

    fn start_motor(&mut self, clk: u64) {
        if self.alarm.is_none() {
            self.alarm = Some(clk + MOTOR_DELAY);
        }
        if self.last_write_clk == 0 {
            // As in VICE: recording starts with the WRITE line at 0, so the
            // first edge written ends a pulse (usually long) from the motor
            // start
            self.last_write_clk = clk;
            self.last_write_bit = 0;
        }
    }

    /// MOTOR line from the processor port: `on` when it is low.
    pub fn set_motor_line(&mut self, on: bool, clk: u64) {
        if on == self.motor_line {
            return;
        }
        self.motor_line = on;
        if on {
            // Cancels a stop in progress
            self.motor_stop_clk = 0;
            if !self.motor {
                self.last_write_clk = 0;
                self.start_motor(clk);
                self.motor = true;
                self.run_start = clk;
            }
        } else if self.motor && self.motor_stop_clk == 0 {
            self.motor_stop_clk = clk + MOTOR_DELAY;
            if self.alarm.is_none() {
                self.alarm = Some(self.motor_stop_clk);
            }
        }
    }

    /// WRITE line from the processor port (bit 3 of $01, 1 also as an
    /// input), on every write to $00/$01: the Datasette only sees the
    /// changes.
    pub fn set_write_line(&mut self, high: bool, clk: u64) {
        if self.write_line == Some(high) {
            return;
        }
        self.write_line = Some(high);
        // datasette_toggle_write_bit: while recording with the motor on
        // every rising edge ends a pulse
        if self.image.is_some() && self.button == Button::Record && self.motor
            && self.last_write_bit == 0 && high
        {
            if self.last_write_clk >= clk {
                self.last_write_clk = clk;
            } else {
                self.write_pulse(clk);
            }
        }
        self.last_write_bit = high as i32;
    }

    /// Writes to the tape the pulse that ended at cycle `clk` (VICE's
    /// bit_write): under 7 cycles it merges with the next one, up to 2046 it
    /// is one byte (the remainder of the division by 8 goes to the next
    /// pulse), beyond that it is a zero followed by the exact duration. The
    /// tape is overwritten from the current position and grows if needed.
    fn write_pulse(&mut self, clk: u64) {
        let mut time = clk - self.last_write_clk;
        self.last_write_clk = clk;
        if time < 7 {
            self.last_write_clk -= time % 8;
            return;
        }
        let tap = self.image.as_mut().unwrap();
        let mut put = |pos: &mut usize, b: u8| {
            match tap.data.get_mut(*pos) {
                Some(d) => *d = b,
                None => tap.data.push(b),
            }
            *pos += 1;
        };
        if time < 255 * 8 + 7 {
            self.last_write_clk -= time % 8;
            put(&mut self.pos, (time / 8) as u8);
        } else {
            put(&mut self.pos, 0);
            if tap.version >= 1 {
                for b in &time.to_le_bytes()[..3] {
                    put(&mut self.pos, *b);
                }
                time &= 0xFF_FFFF;
            }
        }
        tap.changed = true;
        self.cycle_counter += time / 8;
        self.cycle_counter_total = self.cycle_counter_total.max(self.cycle_counter);
    }

    /// One cycle (after the CPU access): true if a pulse arrives on the
    /// FLAG line.
    #[inline]
    pub fn tick(&mut self, clk: u64) -> bool {
        match self.alarm {
            Some(at) if clk >= at => self.alarm_fired(clk, clk - at),
            _ => false,
        }
    }

    /// Tape check (VICE's datasette_read_bit); `late`: cycles of delay
    /// relative to the expected moment.
    fn alarm_fired(&mut self, clk: u64, late: u64) -> bool {
        self.alarm = None;
        if self.motor_stop_clk > 0 {
            if clk >= self.motor_stop_clk {
                self.motor_stop_clk = 0;
                self.motor = false;
            } else {
                self.alarm = Some(self.motor_stop_clk);
            }
        }
        if !self.motor || self.image.is_none() {
            return false;
        }
        // In PLAY the pulse arrives at the start of the gap being read now
        let mut flux = false;
        let (direction, speed) = match self.button {
            Button::Play => {
                flux = self.long_gap_pending == 0;
                (1, DS_V_PLAY)
            }
            // In fast winding the tape speed grows with the reel taking it
            // up
            Button::Forward => (1, self.fast_speed(self.cycle_counter)),
            Button::Rewind => (-1, self.fast_speed(self.cycle_counter_total.saturating_sub(self.cycle_counter))),
            Button::Record | Button::Stop => return false,
        };

        let mut gap;
        if direction + self.last_direction == 0 {
            // Change of direction: the current gap is read again, but only
            // the part already elapsed is travelled
            gap = self.read_gap(direction);
            self.long_gap_pending = self.long_gap_elapsed;
            self.long_gap_elapsed = gap.wrapping_sub(self.long_gap_elapsed);
        }
        if self.long_gap_pending > 0 {
            gap = std::mem::take(&mut self.long_gap_pending);
        } else {
            gap = self.read_gap(direction);
            if gap > 0 {
                self.long_gap_elapsed = 0;
            }
        }
        if gap == 0 {
            // End (or start) of the tape: the Datasette stops
            if direction < 0 {
                // With the tape at the start the counter goes back to 0. Not
                // in VICE: with the speed wobble, forward and backward add up
                // slightly different durations, and the counter stays off
                // (or, below zero, goes haywire)
                self.cycle_counter = 0;
            }
            self.stop();
            return flux;
        }
        if gap > MAX_GAP {
            self.long_gap_pending = gap - MAX_GAP;
            gap = MAX_GAP;
        }
        self.long_gap_elapsed = self.long_gap_elapsed.wrapping_add(gap);
        self.last_direction = direction;
        if self.button == Button::Play && self.sound_volume > 0 {
            self.add_sound(clk - late, gap);
        }
        if direction > 0 {
            self.cycle_counter += gap / 8;
        } else {
            self.cycle_counter = self.cycle_counter.saturating_sub(gap / 8);
        }

        let gap = gap as i64 - late as i64;
        let delay = if gap > 0 { (gap as f64 * (DS_V_PLAY / speed)) as u64 } else { 0 };
        self.alarm = Some(clk + delay);
        if flux {
            self.pulses += 1;
        }
        flux
    }

    /// Tape speed in FF/REW with `counter` units of tape already wound on
    /// the take-up reel (m/s).
    fn fast_speed(&self, counter: u64) -> f64 {
        DS_RPS_FAST / DS_G
            * (4.0 * PI * DS_D * DS_V_PLAY / self.clock_hz * 8.0 * counter as f64
                + 4.0 * PI * PI * DS_R * DS_R).sqrt()
    }

    /// Next gap in cycles in the given direction (1 forward, -1 backward),
    /// with the speed wobble; 0 at the end or start of the tape (VICE's
    /// datasette_read_gap).
    fn read_gap(&mut self, direction: i32) -> u64 {
        let Some(tap) = &self.image else { return 0 };
        let d = &tap.data;
        let version = tap.version;
        let at: usize;
        if direction > 0 {
            if self.pos >= d.len() {
                return 0;
            }
            at = self.pos;
        } else if version == 0 || self.pos < 4 || d[self.pos - 4] != 0 {
            if self.pos == 0 {
                return 0;
            }
            at = self.pos - 1;
        } else {
            match self.find_long_gap_backward() {
                Some(a) => at = a,
                None => return 0,
            }
        }
        let d = &self.image.as_ref().unwrap().data;
        let Some(&b) = d.get(at) else { return 0 };
        let mut step = direction as isize;
        let gap = if version == 0 || b != 0 {
            if b != 0 { b as u64 * 8 } else { ZERO_GAP_DELAY }
        } else {
            if at + 3 >= d.len() {
                return 0;
            }
            step *= 4;
            match u32::from_le_bytes([d[at + 1], d[at + 2], d[at + 3], 0]) {
                0 => ZERO_GAP_DELAY,
                g => g as u64,
            }
        };
        let gap = self.apply_wobble(gap);
        let gap = self.apply_azimuth_error(gap);
        // In VICE the position can become negative (a zero read backwards
        // in the first bytes): from there every read fails
        self.pos = self.pos.saturating_add_signed(step);
        gap
    }

    /// Backwards in a v1 TAP when there is a zero 4 bytes before: the
    /// previous gap may be long (zero + 3 bytes). Like VICE's
    /// read_gap_backward_v1 it goes back until three non-zero bytes in a
    /// row (a point surely outside a long duration) and from there reads
    /// forward again up to 4 bytes before the current position. Returns
    /// where to read.
    fn find_long_gap_backward(&self) -> Option<usize> {
        let d = &self.image.as_ref()?.data;
        let end = self.pos - 4;
        let mut p = end;
        let mut non_zero = 0;
        while non_zero < 3 && p > 0 {
            p -= 1;
            if d[p] != 0 { non_zero += 1 } else { non_zero = 0 }
        }
        while p < end {
            p += if d[p] != 0 { 1 } else { 4 };
        }
        (p < d.len()).then_some(p)
    }

    /// Tape speed wobble (VICE's tape_do_wobble, without the fine speed
    /// tuning, which is 0 by default). As in VICE the phase advances on
    /// every pulse read by a step proportional to the tape length
    /// (`cycle_counter_total`), not to time: the real frequency depends on
    /// the tape. The computations follow the C types (float and double) and
    /// the fused multiply-adds (FMA) into which clang contracts VICE's
    /// expressions: without them, the `tape_wobble` test finds a different
    /// pulse.
    fn apply_wobble(&mut self, gap: u64) -> u64 {
        if self.wobble_frequency == 0 || self.wobble_amplitude == 0 {
            return gap;
        }
        let freq = self.wobble_frequency as f32 / WOBBLE_FREQUENCY_ONE as f32;
        let amplitude = self.wobble_amplitude as f32 / WOBBLE_AMPLITUDE_ONE as f32;
        let cycles = self.cycle_counter_total as f32 / (self.clock_hz as f32 / 1_000_000.0);
        let step = cycles as f64 / (10_000_000_000.0f32 as f64 * (2.0 * std::f64::consts::PI));
        self.wobble_phase = (freq as f64).mul_add(step, self.wobble_phase as f64) as f32;
        if self.wobble_phase as f64 > 2.0 * std::f64::consts::PI {
            self.wobble_phase = (self.wobble_phase as f64 - 2.0 * std::f64::consts::PI) as f32;
        }
        let factor = self.wobble_phase.sin().mul_add(amplitude, 1.0);
        let new_gap_f = factor.mul_add(gap as f32, self.wobble_rest);
        let new_gap = ((new_gap_f + 0.5) as i32).max(1);
        self.wobble_rest = new_gap_f - new_gap as f32;
        new_gap as u64
    }
}

impl Datasette {
    /// Azimuth error (VICE's tape_do_misalignment): the gap, in thousandths
    /// of a cycle, moves by a random value in ±`azimuth_error` and is
    /// rounded to the cycle, carrying the remainder to the next one. VICE
    /// 3.10 has a bug: the negative random value ends up in a 64-bit
    /// unsigned integer and becomes about +4.3 million cycles, so with
    /// `-dstapeerror` the tape no longer loads. Here the shift is signed, as
    /// VICE intended. The generator is VICE's (PCG) with a fixed initial
    /// state: same tapes, same errors.
    fn apply_azimuth_error(&mut self, gap: u64) -> u64 {
        if self.azimuth_error == 0 {
            return gap;
        }
        let e = self.azimuth_error as i64;
        let error = (self.random() as i64 * (2 * e + 1)) >> 32;
        let exact = gap as i64 * AZIMUTH_ERROR_ONE as i64 + (error - e) + self.azimuth_rest;
        let new_gap = (exact + AZIMUTH_ERROR_ONE as i64 / 2).div_euclid(AZIMUTH_ERROR_ONE as i64).max(1);
        self.azimuth_rest = exact - new_gap * AZIMUTH_ERROR_ONE as i64;
        new_gap as u64
    }

    /// 32-bit random number (PCG-XSH-RR 32/64, VICE's rand_uint32).
    fn random(&mut self) -> u32 {
        let prev = self.rng;
        self.rng = prev.wrapping_mul(PCG_MULTIPLIER).wrapping_add(PCG_INCREMENT);
        let base = ((prev ^ (prev >> 18)) >> 27) as u32;
        base.rotate_right((prev >> 59) as u32)
    }
}

const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const PCG_INCREMENT: u64 = 1;

impl Default for Datasette {
    fn default() -> Self {
        Self::new()
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

use crate::snapshot::{impl_state, impl_state_enum};

impl_state!(Tap { version, system, video, data, changed });

impl_state_enum!(Button { Stop, Play, Forward, Rewind, Record });

impl_state!(Datasette {
    image, pos, button, motor, motor_stop_clk, motor_line, alarm, long_gap_pending, long_gap_elapsed,
    last_direction, cycle_counter, cycle_counter_total, counter_offset, last_write_clk, last_write_bit,
    write_line, wobble_amplitude, wobble_frequency, wobble_phase, wobble_rest, azimuth_error, azimuth_rest,
    rng, sound_volume, sound_sign, sound_level, sound_from, pulses, run_start,
} skip { sound_edges, clock_hz });

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(version: u8, data: &[u8]) -> Tap {
        Tap { version, data: data.to_vec(), ..Tap::default() }
    }

    fn datasette(t: Tap) -> Datasette {
        let mut ds = Datasette::new();
        ds.set_wobble(0, 0);
        ds.insert(t);
        ds
    }

    /// Cycles of the pulses from `from` to `until` (excluded): jumps from
    /// one tape check to the next, like calling `tick` on every cycle.
    fn pulses_between(ds: &mut Datasette, from: u64, until: u64) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(at) = ds.alarm {
            let at = at.max(from);
            if at >= until {
                break;
            }
            if ds.tick(at) {
                out.push(at);
            }
        }
        out
    }

    /// Fast winding until the Datasette stops, with no pulses on the FLAG
    /// line; returns the cycle of the stop.
    fn wind_until_stop(ds: &mut Datasette) -> u64 {
        while ds.button() != Button::Stop {
            let at = ds.alarm.expect("tape stopped before the end");
            assert!(!ds.tick(at), "pulse during fast winding");
            if ds.button() == Button::Stop {
                return at;
            }
        }
        unreachable!()
    }

    /// Cycles of the pulses from cycle 0, with the motor on and PLAY down.
    fn pulses(ds: &mut Datasette, until: u64) -> Vec<u64> {
        pulses_between(ds, 0, until)
    }

    #[test]
    fn pulses_follow_gaps_after_motor_delay() {
        let mut ds = datasette(tap(1, &[0x30, 0x42, 0x00, 0x10, 0x27, 0x00, 0x30]));
        ds.play(0);
        ds.set_motor_line(true, 0);
        let p = pulses(&mut ds, 60_000);
        // First pulse after the motor delay, then 0x30*8, 0x42*8,
        // 10000 cycles (version 1 long pulse), 0x30*8, end
        assert_eq!(p, [32_000, 32_384, 32_912, 42_912, 43_296]);
        assert_eq!(ds.button(), Button::Stop);
    }

    #[test]
    fn motor_stop_is_delayed_and_can_be_cancelled() {
        let mut ds = datasette(tap(1, &[0xFF; 100]));
        ds.play(0);
        ds.set_motor_line(true, 0);
        pulses(&mut ds, 40_000);
        // Motor off: it keeps running for 32000 cycles
        ds.set_motor_line(false, 40_000);
        assert!(ds.motor());
        let before = ds.position().0;
        (40_000..71_999).for_each(|c| { ds.tick(c); });
        assert!(ds.motor() && ds.position().0 > before);
        (71_999..80_000).for_each(|c| { ds.tick(c); });
        assert!(!ds.motor());
    }

    /// Writes pulses of duration `gaps` with the Datasette in RECORD: the
    /// first edge ends the pulse from the motor start.
    fn record(ds: &mut Datasette, start: u64, gaps: &[u64]) -> u64 {
        ds.press(Button::Record, start);
        ds.set_write_line(false, start);
        ds.set_motor_line(true, start);
        let mut clk = start;
        for &g in gaps {
            clk += g;
            ds.set_write_line(true, clk);
            ds.set_write_line(false, clk + 3);
        }
        clk
    }

    #[test]
    fn record_writes_gaps_like_vice() {
        let mut ds = datasette(Tap::blank());
        // 400: one byte; 403 + 5: the remainder (3) goes to the next pulse;
        // 4: under 7 cycles it merges with the next one; 3000: long
        record(&mut ds, 1000, &[400, 403, 405, 4, 300, 3000]);
        let t = ds.image.as_ref().unwrap();
        assert!(t.changed);
        assert_eq!(t.data, [50, 50, 51, 38, 0, 0xB8, 0x0B, 0x00]);
        // The same tape read back gives the same pulses (except remainders)
        let mut ds = datasette(t.clone());
        ds.play(0);
        ds.set_motor_line(true, 0);
        let p = pulses(&mut ds, 50_000);
        let gaps: Vec<u64> = p.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(gaps, [400, 400, 408, 304, 3000]);
    }

    #[test]
    fn record_overwrites_from_current_position() {
        let mut ds = datasette(tap(1, &[0x30; 10]));
        ds.play(0);
        ds.set_motor_line(true, 0);
        pulses(&mut ds, 32_000 + 3 * 0x180 + 1);
        ds.stop();
        assert_eq!(ds.position().0, 4);
        // Motor stopped before pressing RECORD, as with the KERNAL
        ds.set_motor_line(false, 33_200);
        pulses_between(&mut ds, 33_200, 70_000);
        assert!(!ds.motor());
        record(&mut ds, 70_000, &[800, 800]);
        let t = ds.image.as_ref().unwrap();
        assert_eq!(t.data.len(), 10);
        assert_eq!(&t.data[..], &[0x30, 0x30, 0x30, 0x30, 0x64, 0x64, 0x30, 0x30, 0x30, 0x30]);
    }

    #[test]
    fn record_needs_record_button_and_motor() {
        let mut ds = datasette(Tap::blank());
        ds.play(0);
        ds.set_motor_line(true, 0);
        ds.set_write_line(false, 100);
        ds.set_write_line(true, 900);
        assert!(!ds.image.as_ref().unwrap().changed);
        // Without a tape RECORD cannot be pressed
        let mut empty = Datasette::new();
        empty.press(Button::Record, 0);
        assert_eq!(empty.button(), Button::Stop);
    }

    #[test]
    fn fast_forward_and_rewind_cross_the_tape() {
        let data = [0x40; 20_000];
        let mut ds = datasette(tap(1, &data));
        ds.set_motor_line(true, 0);
        ds.press(Button::Forward, 0);
        assert!(ds.sense());
        // The tape reaches the end much earlier than in PLAY (20000 × 512 cycles)
        let end = wind_until_stop(&mut ds);
        assert_eq!(ds.position().0, data.len());
        let play_time = data.len() as u64 * 0x40 * 8;
        assert!(end < play_time / 10, "FF too slow: {end} cycles");
        assert!(ds.counter() > 0);

        ds.press(Button::Rewind, end);
        let start = wind_until_stop(&mut ds);
        assert_eq!(ds.position().0, 0);
        assert_eq!(ds.counter(), 0);
        // FF starts slow (the take-up reel is empty) and speeds up, REW
        // from the end does the same in reverse: the times are close
        let (ff, rew) = (end, start - end);
        assert!(ff.abs_diff(rew) < ff / 100, "FF {ff} REW {rew}");
    }

    #[test]
    fn counter_back_to_zero_with_wobble() {
        // With the wobble FF and REW add up slightly different durations
        // (and the truncation to 1/8 amplifies the difference): the counter
        // drifts one way or the other depending on the tape. With 40000
        // pulses of $20 it would end 30476 units ahead (a quarter of a
        // second, below the counter resolution but not the REW speed's),
        // with 20000 of $30 below zero (the counter would show random numbers)
        for (len, b) in [(40_000, 0x20u8), (20_000, 0x30)] {
            let mut ds = Datasette::new();
            ds.insert(tap(1, &vec![b; len]));
            ds.set_motor_line(true, 0);
            ds.press(Button::Forward, 0);
            let end = wind_until_stop(&mut ds);
            assert!(ds.counter() > 0);
            ds.press(Button::Rewind, end);
            wind_until_stop(&mut ds);
            assert_eq!((ds.position().0, ds.cycle_counter, ds.counter()), (0, 0, 0), "{len} pulses of ${b:02X}");
        }
    }

    #[test]
    fn rewind_over_long_gaps() {
        // Long pulses (zero + 3 bytes) among short pulses: backwards they
        // must be crossed as a single gap
        let data = [0x30, 0x31, 0x32, 0x00, 0x10, 0x27, 0x00, 0x33, 0x00, 0x20, 0x4E, 0x00, 0x34];
        let mut ds = datasette(tap(1, &data));
        ds.set_motor_line(true, 0);
        ds.press(Button::Forward, 0);
        let end = wind_until_stop(&mut ds);
        assert_eq!(ds.position().0, data.len());
        let mut seen = vec![ds.position().0];
        ds.press(Button::Rewind, end);
        while ds.button() != Button::Stop {
            ds.tick(ds.alarm.unwrap());
            if *seen.last().unwrap() != ds.position().0 {
                seen.push(ds.position().0);
            }
        }
        assert_eq!(seen, [13, 12, 8, 7, 3, 2, 1, 0]);
    }

    #[test]
    fn fast_forward_skips_pulses() {
        let mut ds = datasette(tap(1, &[0x40; 2000]));
        ds.play(0);
        ds.set_motor_line(true, 0);
        assert_eq!(pulses_between(&mut ds, 0, 40_000).len(), 16);
        ds.press(Button::Forward, 40_000);
        pulses_between(&mut ds, 40_000, 100_000);
        ds.stop();
        let at = ds.position().0;
        assert!(at > 100, "FF stopped at {at}");
        ds.play(100_000);
        // From here in PLAY only the remaining pulses arrive
        let rest = pulses_between(&mut ds, 100_000, 10_000_000);
        assert_eq!(rest.len(), 2000 - at + 1);
        assert_eq!(ds.button(), Button::Stop);
    }

    #[test]
    fn wobble_changes_gaps_by_half_percent() {
        // As in VICE the phase advances with the tape length: with 20000
        // pulses it covers more than one period
        let mut ds = Datasette::new();
        ds.insert(tap(1, &[0xFA; 20_000]));
        ds.play(0);
        ds.set_motor_line(true, 0);
        let p = pulses(&mut ds, 50_000_000);
        assert_eq!(p.len(), 20_001);
        let gaps: Vec<u64> = p.windows(2).map(|w| w[1] - w[0]).collect();
        let (lo, hi) = (*gaps.iter().min().unwrap(), *gaps.iter().max().unwrap());
        assert!(lo < 2000 && hi > 2000, "no wobble: {lo}-{hi}");
        assert!(lo >= 1989 && hi <= 2011, "wobble beyond 0.5%: {lo}-{hi}");
    }

    #[test]
    fn azimuth_error_moves_gaps_but_keeps_time() {
        let mut ds = datasette(tap(1, &[0x40; 2000]));
        ds.set_azimuth_error(3000);
        ds.play(0);
        ds.set_motor_line(true, 0);
        let p = pulses(&mut ds, 2_000_000);
        let gaps: Vec<u64> = p.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(gaps.len(), 2000);
        let (lo, hi) = (*gaps.iter().min().unwrap(), *gaps.iter().max().unwrap());
        assert!(lo < 512 && hi > 512, "no error: {lo}-{hi}");
        assert!(lo >= 508 && hi <= 516, "error beyond ±3 cycles (+ remainder): {lo}-{hi}");
        // The remainder goes to the next pulse: the errors add up like a
        // random walk (about ±77 cycles over 2000 pulses), without moving the mean
        let total: u64 = gaps.iter().sum();
        assert!(total.abs_diff(2000 * 512) <= 300, "drift {total}");
        // Same tape, same errors
        let mut again = datasette(tap(1, &[0x40; 2000]));
        again.set_azimuth_error(3000);
        again.play(0);
        again.set_motor_line(true, 0);
        assert_eq!(pulses(&mut again, 2_000_000), p);
    }

    #[test]
    fn pcg_matches_vice() {
        // First values of VICE's rand_uint32 (compiled lib.c) from the
        // initial state, without a seed
        let mut ds = Datasette::new();
        let got: Vec<u32> = (0..4).map(|_| ds.random()).collect();
        assert_eq!(got, [0xE4C14788, 0x379C6516, 0x5C4AB3BB, 0x601D23E0]);
    }

    #[test]
    fn sound_is_a_square_wave_per_pulse() {
        let mut ds = datasette(tap(1, &[0x40; 100]));
        ds.set_sound(Some(1000));
        ds.play(0);
        ds.set_motor_line(true, 0);
        // Pulses every 512 cycles from 32000: one sample every 64 cycles
        pulses(&mut ds, 40_000);
        let mut buf = vec![0i16; 40_000 / 64];
        ds.mix_sound(&mut buf, 1, 40_000);
        let first = 32_000 / 64;
        assert!(buf[..first].iter().all(|&v| v == 0));
        // 4 samples at +1000, 4 at -1000 for every pulse
        for k in 0..8 {
            assert_eq!(&buf[first + 8 * k..first + 8 * k + 8], &[1000, 1000, 1000, 1000, -1000, -1000, -1000, -1000]);
        }
        // Stereo: the same on both channels; with the tape stopped it is silent
        ds.stop();
        let mut st = vec![0i16; 2 * 100];
        ds.mix_sound(&mut st, 2, 46_400);
        assert!(st.chunks(2).all(|c| c[0] == c[1]));
        assert_eq!(st[st.len() - 1], 0);
    }

    #[test]
    fn tap_round_trip() {
        let t = Tap { version: 1, system: 0, video: 0, data: vec![1, 2, 3], changed: true };
        let back = Tap::from_bytes(&t.to_bytes()).unwrap();
        assert_eq!((back.version, back.data), (1, vec![1, 2, 3]));
        assert_eq!(Tap::blank().to_bytes().len(), 24);
    }

    #[test]
    fn t64_with_wrong_end_address() {
        let mut img = vec![0u8; 0x60 + 5];
        img[..3].copy_from_slice(b"C64");
        img[0x22] = 1;
        let e = &mut img[0x40..0x60];
        e[0] = 1;
        e[1] = 0x82;
        e[2..4].copy_from_slice(&0x0801u16.to_le_bytes());
        e[4..6].copy_from_slice(&0xC3C6u16.to_le_bytes());
        e[8..12].copy_from_slice(&0x60u32.to_le_bytes());
        e[16..20].copy_from_slice(b"GAME");
        img[0x60..].copy_from_slice(&[1, 2, 3, 4, 5]);
        let (name, prg) = t64_first_prg(&img).unwrap();
        assert_eq!(name, "GAME");
        assert_eq!(prg, [0x01, 0x08, 1, 2, 3, 4, 5]);
    }
}
