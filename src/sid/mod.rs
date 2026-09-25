// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>, and from VICE's
// sound.c (Teemu Rantanen, Marco van den Heuvel, Ettore Perazzoli).
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! SID 6581/8580: a port of reSID 1.0 as shipped in VICE 3.10 (x64sc,
//! ReSID engine with the `filter8580new` filter), cycle by cycle.
//!
//! - Digital part (always active, visible to the program): oscillators,
//!   noise register, combined waveforms from the sampled tables,
//!   envelopes, OSC3/ENV3 and the data bus (write-only registers read as
//!   the last value seen on the bus, which fades after a while).
//! - Analog part (only with audio on): waveform and envelope DACs with
//!   the chip's imperfections, transistor-level filter and mixer, C64
//!   output stage, resampling with a Kaiser FIR filter from 985 kHz to the
//!   audio device rate. Without audio (debugger, tests, turbo) it is not
//!   computed: it costs as much as the whole rest of the emulator.
//!
//! The SID advances with the CPU clock: `tick` counts the cycles and the
//! actual emulation catches up (`catch_up`) before every register access
//! and at the end of each frame.

mod dac;
mod envelope;
mod extfilt;
mod filter;
mod wave;

use envelope::Envelope;
use extfilt::ExternalFilter;
use filter::{Filter, ModelFilter};
use wave::WaveformGenerator;

use crate::c64::CLOCK_HZ;

/// Chip model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// C64 "breadbin" and early C64C: filter with the 6581 distortions,
    /// DC component that makes samples via $D418 audible.
    Mos6581 = 0,
    Mos8580 = 1,
}

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Model::Mos6581 => "6581",
            Model::Mos8580 => "8580",
        }
    }
}

/// Lifetime of the value on the data bus (cycles), measured on real
/// chips (testprogs/SID/bitfade/delayfrq0.prg).
fn databus_ttl(model: Model) -> i32 {
    match model {
        Model::Mos6581 => 0x1D00,
        Model::Mos8580 => 0xA2000,
    }
}

// ── Voice ─────────────────────────────────────────────────────────────────────

struct Voice {
    wave: WaveformGenerator,
    env: Envelope,
}

impl Voice {
    fn new() -> Self {
        Self { wave: WaveformGenerator::new(), env: Envelope::new() }
    }

    /// Multiplying DAC output: waveform (minus its "zero") times
    /// envelope, about ±2048*255.
    #[inline]
    fn output(&self, model: Model) -> i32 {
        // The 6581 waveform DAC has its zero at 0x380 instead of
        // 0x800: hence the DC component of each voice
        let wave_zero = match model {
            Model::Mos6581 => 0x380,
            Model::Mos8580 => 0x9E0,
        };
        (self.wave.output() - wave_zero) * self.env.output(model)
    }
}

// ── Resampling ───────────────────────────────────────────────────────────────

/// 1 MHz samples kept for the FIR filter (duplicated so they are
/// contiguous).
const RINGSIZE: usize = 1 << 14;
const RINGMASK: usize = RINGSIZE - 1;
const FIR_N: f64 = 125.0;
const FIR_RES: f64 = 285.0;
const FIR_SHIFT: u32 = 15;
const FIXP_SHIFT: u32 = 16;
const FIXP_MASK: i32 = 0xFFFF;

/// Passband (percentage of half the sampling rate) and FIR filter
/// gain: the VICE defaults.
const PASSBAND_PERCENT: f64 = 90.0;
const FIR_GAIN: f64 = 0.97;

/// FIR filter for the output rate: `res` tables of `n` coefficients
/// for the fractional sample positions (reSID, SAMPLE_RESAMPLE).
struct Fir {
    rate: u32,
    cycles_per_sample: i32,
    n: usize,
    res: usize,
    table: Vec<i16>,
}

/// Zeroth-order modified Bessel function (Kaiser window).
fn i0(x: f64) -> f64 {
    const I0E: f64 = 1e-6;
    let (mut sum, mut u, mut n) = (1.0f64, 1.0f64, 1.0f64);
    let halfx = x / 2.0;
    loop {
        let temp = halfx / n;
        n += 1.0;
        u *= temp * temp;
        sum += u;
        if u < I0E * sum {
            break;
        }
    }
    sum
}

impl Fir {
    fn new(rate: u32) -> Result<Self, String> {
        let clock_freq = CLOCK_HZ as f64;
        let sample_freq = rate as f64;
        let pass_freq = rate as f64 * PASSBAND_PERCENT / 200.0;
        if (FIR_N * clock_freq / sample_freq) as i32 >= RINGSIZE as i32 {
            return Err(format!("audio rate {rate} Hz too low"));
        }
        let cycles_per_sample = (clock_freq / sample_freq * (1 << FIXP_SHIFT) as f64 + 0.5) as i32;

        // Kaiser filter: 16 bits of stopband attenuation, transition
        // band from pass_freq to sample_freq/2
        let pi = std::f64::consts::PI;
        let a = -20.0 * (1.0 / (1 << 16) as f64).log10();
        let dw = (1.0 - 2.0 * pass_freq / sample_freq) * pi * 2.0;
        let wc = pi;
        let beta = 0.1102 * (a - 8.7);
        let i0beta = i0(beta);
        let mut n = ((a - 7.95) / (2.285 * dw) + 0.5) as i32;
        n += n & 1;

        let f_samples_per_cycle = sample_freq / clock_freq;
        let f_cycles_per_sample = clock_freq / sample_freq;
        let fir_n = ((n as f64 * f_cycles_per_sample) as i32 + 1) | 1;
        let bits = (FIR_RES / f_cycles_per_sample).ln() / 2.0f64.ln();
        let fir_res = 1usize << bits.ceil() as i32;

        let fir_n = fir_n as usize;
        let half = (fir_n / 2) as i32;
        let mut table = vec![0i16; fir_n * fir_res];
        for i in 0..fir_res {
            let fir_offset = i * fir_n + fir_n / 2;
            let j_offset = i as f64 / fir_res as f64;
            for j in -half..=half {
                let jx = j as f64 - j_offset;
                let wt = wc * jx / f_cycles_per_sample;
                let temp = jx / half as f64;
                let kaiser = if temp.abs() <= 1.0 { i0(beta * (1.0 - temp * temp).sqrt()) / i0beta } else { 0.0 };
                let sincwt = if wt.abs() >= 1e-6 { wt.sin() / wt } else { 1.0 };
                let val = (1 << FIR_SHIFT) as f64 * FIR_GAIN * f_samples_per_cycle * wc / pi * sincwt * kaiser;
                table[(fir_offset as i32 + j) as usize] = val.round() as i16;
            }
        }
        Ok(Self { rate, cycles_per_sample, n: fir_n, res: fir_res, table })
    }
}

#[inline]
fn dot(a: &[i16], b: &[i16]) -> i32 {
    a.iter().zip(b).fold(0i32, |s, (&x, &y)| s.wrapping_add(x as i32 * y as i32))
}

#[inline]
fn clip(v: i32) -> i16 {
    v.clamp(-32768, 32767) as i16
}

/// Mono mix of two audio outputs as in VICE (`sound_audio_mix`): opposite
/// signs add up, equal signs approach the maximum without exceeding it.
pub fn audio_mix(a: i16, b: i16) -> i16 {
    let (a, b) = (a as i32, b as i32);
    if a == 0 {
        return b as i16;
    }
    if b == 0 {
        return a as i16;
    }
    if (a > 0) != (b > 0) {
        return (a + b) as i16;
    }
    if a > 0 {
        ((a + b) - (a * b / 32768)) as i16
    } else {
        (-((-a + -b) - (-a * -b / 32768))) as i16
    }
}

/// Possible addresses of the second SID, as in VICE: $D420-$D7E0 and
/// $DE00-$DFE0 in steps of $20.
pub fn valid_sid2_address(a: u16) -> bool {
    a & 0x1F == 0 && ((0xD420..=0xD7E0).contains(&a) || (0xDE00..=0xDFE0).contains(&a))
}

// ── SID ───────────────────────────────────────────────────────────────────────

pub struct Sid {
    /// Last values written to the registers (for the debugger).
    pub registers: [u8; 32],
    model: Model,
    /// 8580 with EXT IN grounded through a resistor ("digiboost"): samples
    /// via $D418 are audible on the 8580 too.
    digiboost: bool,
    voices: [Voice; 3],
    filter: Filter,
    extfilt: ExternalFilter,
    bus_value: u8,
    bus_value_ttl: i32,
    /// CPU cycles not yet emulated.
    pending: u32,
    // Resampling state: position of the next sample in 16.16,
    // recent 1 MHz samples
    sample_offset: i32,
    sample_index: u32,
    ring: Vec<i16>,
    /// Samples produced and not yet taken.
    samples: Vec<i16>,
    /// Audio on: FIR filter for the output rate.
    fir: Option<Fir>,
    /// Filter tables for the model, built the first time audio is
    /// switched on.
    tables: Option<&'static ModelFilter>,
    /// Filter output at every cycle, before the output stage (like
    /// VICE's `-residrawoutput`), for comparisons.
    pub raw_capture: Option<Vec<i16>>,
    /// Audio just switched back on: the next cycle realigns the output stage.
    resume: bool,
}

impl Sid {
    pub fn new() -> Self {
        let mut sid = Self {
            registers: [0u8; 32],
            model: Model::Mos6581,
            digiboost: false,
            voices: [Voice::new(), Voice::new(), Voice::new()],
            filter: Filter::new(),
            extfilt: ExternalFilter::new(),
            bus_value: 0,
            bus_value_ttl: 0,
            pending: 0,
            sample_offset: 0,
            sample_index: 0,
            ring: vec![0; RINGSIZE * 2],
            samples: Vec::with_capacity(2048),
            fir: None,
            tables: None,
            raw_capture: None,
            resume: false,
        };
        sid.set_model(Model::Mos6581, false);
        sid
    }

    pub fn model(&self) -> Model {
        self.model
    }

    pub fn digiboost(&self) -> bool {
        self.digiboost
    }

    /// Changes model. `digiboost` (8580 only): EXT IN connected to the mixer
    /// at a fixed level, like the hardware mod used for samples.
    pub fn set_model(&mut self, model: Model, digiboost: bool) {
        self.catch_up();
        self.model = model;
        self.digiboost = digiboost && model == Model::Mos8580;
        for v in &mut self.voices {
            v.wave.set_model(model);
        }
        // Cutoff DAC bias as in VICE: 500 mV for the 6581
        let (mask, ext_in, bias) = match (model, self.digiboost) {
            (Model::Mos6581, _) => (0x07, 0, 0.5),
            (Model::Mos8580, false) => (0x07, 0, 0.0),
            (Model::Mos8580, true) => (0x0F, -32768, 0.0),
        };
        self.filter.set_model(model, mask, ext_in, bias);
        if self.tables.is_some() {
            self.load_tables();
        }
    }

    /// Chip reset (RES line, tied to the C64 reset).
    pub fn reset(&mut self) {
        self.catch_up();
        for v in &mut self.voices {
            v.wave.reset();
            v.env.reset();
        }
        self.filter.reset();
        if let Some(t) = self.tables {
            self.filter.refresh(t);
        }
        self.extfilt.reset();
        self.bus_value = 0;
        self.bus_value_ttl = 0;
        self.registers = [0; 32];
    }

    fn load_tables(&mut self) {
        let t = filter::tables(self.model);
        self.tables = Some(t);
        self.filter.refresh(t);
    }

    /// Audio on at `rate` (Hz), or off. The first time it is switched on
    /// the filter tables are built (a few tenths of a second).
    pub fn set_audio(&mut self, rate: Option<u32>) -> Result<(), String> {
        self.catch_up();
        match rate {
            None => self.fir = None,
            Some(rate) => {
                if self.fir.as_ref().is_some_and(|f| f.rate == rate) {
                    return Ok(());
                }
                let fir = Fir::new(rate)?;
                if self.tables.is_none() {
                    self.load_tables();
                }
                if self.fir.is_none() {
                    // With audio off (turbo, maximum speed) the analog part
                    // was not emulated: the old 1 MHz samples are discarded
                    // and on the first cycle the output stage restarts from
                    // the current value, with the previous average level. So
                    // resuming has no jump from a stale state.
                    self.ring.fill(0);
                    self.resume = true;
                }
                self.fir = Some(fir);
            }
        }
        Ok(())
    }

    /// After restoring a snapshot into this SID: audio at the rate of the
    /// previous machine, without emulating the pending cycles.
    pub fn inherit_audio(&mut self, rate: Option<u32>) -> Result<(), String> {
        self.fir = rate.map(Fir::new).transpose()?;
        if self.fir.is_some() {
            self.load_tables();
        }
        Ok(())
    }

    /// Aligns the resampling phase to that of `other`: two SIDs playing
    /// together produce the same number of samples per frame.
    pub fn sync_sampling(&mut self, other: &Sid) {
        self.catch_up();
        self.sample_offset = other.sample_offset;
    }

    /// Audio rate, if on.
    pub fn audio_rate(&self) -> Option<u32> {
        self.fir.as_ref().map(|f| f.rate)
    }

    /// One clock cycle (φ2).
    #[inline]
    pub fn tick(&mut self) {
        self.pending += 1;
    }

    /// Emulates the cycles accumulated by `tick`.
    fn catch_up(&mut self) {
        let n = std::mem::take(&mut self.pending);
        if n == 0 {
            return;
        }
        match (&self.fir, self.tables) {
            (Some(_), Some(t)) => self.clock_resample(n as i32, t),
            _ => self.clock_digital(n),
        }
    }

    /// Digital part only: envelopes in blocks, oscillators cycle by
    /// cycle (combined waveforms modify accumulator and noise).
    fn clock_digital(&mut self, n: u32) {
        for v in &mut self.voices {
            v.env.clock_n(n);
        }
        for _ in 0..n {
            self.clock_waves();
        }
        if self.bus_value_ttl > 0 {
            self.bus_value_ttl -= n as i32;
            if self.bus_value_ttl <= 0 {
                self.bus_value_ttl = 0;
                self.bus_value = 0;
            }
        }
    }

    /// Oscillators, sync and waveform outputs for one cycle.
    #[inline]
    fn clock_waves(&mut self) {
        for v in &mut self.voices {
            v.wave.clock();
        }
        // Hard sync: voice 1 → 2 → 3 → 1, after all voices have been clocked
        for i in 0..3 {
            let (dest, src) = ((i + 1) % 3, (i + 2) % 3);
            let w = &self.voices[i].wave;
            if w.sync_dest(self.voices[dest].wave.sync, self.voices[src].wave.msb_rising) {
                self.voices[dest].wave.accumulator = 0;
            }
        }
        // Voice i is modulated by voice i-1 (ring modulation)
        for i in 0..3 {
            let src = self.voices[(i + 2) % 3].wave.accumulator;
            self.voices[i].wave.set_waveform_output(src);
        }
    }

    /// One full cycle, like reSID's `SID::clock()`: returns the 16-bit
    /// audio output.
    #[inline]
    fn clock_cycle(&mut self, t: &ModelFilter) -> i16 {
        for v in &mut self.voices {
            v.env.clock();
        }
        self.clock_waves();
        let m = self.model;
        let (o1, o2, o3) = (self.voices[0].output(m), self.voices[1].output(m), self.voices[2].output(m));
        self.filter.clock(t, o1, o2, o3);
        let out = self.filter.output(t);
        if let Some(raw) = &mut self.raw_capture {
            raw.push(out);
        }
        if self.resume {
            self.extfilt.resume(out);
            self.resume = false;
        }
        self.extfilt.clock(out);
        if self.bus_value_ttl > 0 {
            self.bus_value_ttl -= 1;
            if self.bus_value_ttl == 0 {
                self.bus_value = 0;
            }
        }
        clip(self.extfilt.output())
    }

    /// `delta_t` cycles with audio: each cycle goes into the 1 MHz sample
    /// queue, and every output period the FIR filter derives one sample
    /// from it, interpolating between the two tables nearest the fractional
    /// position (reSID, `clock_resample`).
    fn clock_resample(&mut self, mut delta_t: i32, t: &'static ModelFilter) {
        let Some(fir) = self.fir.take() else { return };
        let scale = match self.model {
            Model::Mos6581 => 3,
            Model::Mos8580 => 5,
        };
        loop {
            let next_sample_offset = self.sample_offset + fir.cycles_per_sample;
            let delta_t_sample = (next_sample_offset >> FIXP_SHIFT).min(delta_t);
            let mut idx = self.sample_index as usize;
            for _ in 0..delta_t_sample {
                let s = self.clock_cycle(t);
                self.ring[idx] = s;
                self.ring[idx + RINGSIZE] = s;
                idx = (idx + 1) & RINGMASK;
            }
            self.sample_index = idx as u32;
            delta_t -= delta_t_sample;
            if delta_t == 0 {
                self.sample_offset -= delta_t_sample << FIXP_SHIFT;
                break;
            }
            self.sample_offset = next_sample_offset & FIXP_MASK;

            let pos = self.sample_offset as usize * fir.res;
            let mut fir_offset = pos >> FIXP_SHIFT;
            let fir_offset_rmd = (pos & FIXP_MASK as usize) as u32;
            let mut start = idx + RINGSIZE - fir.n - 1;
            let v1 = dot(&self.ring[start..start + fir.n], &fir.table[fir_offset * fir.n..][..fir.n]);
            fir_offset += 1;
            if fir_offset == fir.res {
                fir_offset = 0;
                start += 1;
            }
            let v2 = dot(&self.ring[start..start + fir.n], &fir.table[fir_offset * fir.n..][..fir.n]);
            // Linear interpolation between the two tables (in unsigned
            // arithmetic, like reSID)
            let v = v1.wrapping_add((fir_offset_rmd.wrapping_mul(v2.wrapping_sub(v1) as u32) >> FIXP_SHIFT) as i32);
            // Output gain: scale/2
            self.samples.push(clip((scale * (v >> FIR_SHIFT)) / 2));
        }
        self.fir = Some(fir);
    }

    /// Read without side effects (debugger): the SID may be a few
    /// cycles behind.
    pub fn read(&self, reg: u8) -> u8 {
        match reg & 0x1F {
            0x19 | 0x1A => 0xFF, // potentiometers not connected
            0x1B => self.voices[2].wave.read_osc(),
            0x1C => self.voices[2].env.env3,
            _ => self.bus_value,
        }
    }

    /// CPU read: write-only registers return the value on the data bus,
    /// which OSC3 and ENV3 refresh. The potentiometers (no paddle or
    /// mouse connected) return $FF without touching the bus, as in VICE.
    pub fn read_mut(&mut self, reg: u8) -> u8 {
        self.catch_up();
        let r = reg & 0x1F;
        match r {
            0x19 | 0x1A => return 0xFF,
            0x1B | 0x1C => {
                self.bus_value = self.read(r);
                self.bus_value_ttl = databus_ttl(self.model);
            }
            _ => {}
        }
        self.bus_value
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        self.catch_up();
        let r = (reg & 0x1F) as usize;
        self.registers[r] = val;
        self.bus_value = val;
        self.bus_value_ttl = databus_ttl(self.model);
        let t = self.tables;
        if r < 21 {
            let i = r / 7;
            let src = self.voices[(i + 2) % 3].wave.accumulator;
            let v = &mut self.voices[i];
            match r % 7 {
                0 => v.wave.write_freq_lo(val),
                1 => v.wave.write_freq_hi(val),
                2 => v.wave.write_pw_lo(val),
                3 => v.wave.write_pw_hi(val),
                4 => {
                    v.wave.write_control(val, src);
                    v.env.write_control(val);
                }
                5 => v.env.write_attack_decay(val),
                _ => v.env.write_sustain_release(val),
            }
        } else {
            match r {
                0x15 => self.filter.write_fc_lo(val, t),
                0x16 => self.filter.write_fc_hi(val, t),
                0x17 => self.filter.write_res_filt(val),
                0x18 => self.filter.write_mode_vol(val),
                _ => {}
            }
        }
    }

    /// Brings the SID up to the current cycle and moves the produced samples
    /// into `buf` (there are none with audio off).
    pub fn take_samples(&mut self, buf: &mut Vec<i16>) {
        self.catch_up();
        buf.append(&mut self.samples);
    }

    /// Current OSC3 and ENV3 (debugger).
    pub fn osc3_env3(&self) -> (u8, u8) {
        (self.voices[2].wave.read_osc(), self.voices[2].env.env3)
    }
}

impl Default for Sid {
    fn default() -> Self {
        Self::new()
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

use crate::snapshot::{impl_state, impl_state_enum};

impl_state_enum!(Model { Mos6581, Mos8580 });

impl_state!(Voice { wave, env });

// Audio configuration and tables are excluded, as they belong to the
// frontend: `C64::restore` switches audio back on for the restored machine.
impl_state!(Sid {
    registers, model, digiboost, voices, filter, extfilt, bus_value, bus_value_ttl, pending,
    sample_offset, sample_index, ring, samples,
} skip { fir, tables, raw_capture, resume });

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sid: &mut Sid, cycles: u32) {
        for _ in 0..cycles { sid.tick(); }
    }

    #[test]
    fn one_second_gives_rate_samples() {
        for rate in [44_100, 48_000] {
            let mut sid = Sid::new();
            sid.set_audio(Some(rate)).unwrap();
            let mut buf = Vec::new();
            for _ in 0..50 {
                run(&mut sid, CLOCK_HZ as u32 / 50);
                sid.take_samples(&mut buf);
            }
            run(&mut sid, CLOCK_HZ as u32 % 50);
            sid.take_samples(&mut buf);
            assert!((buf.len() as i64 - rate as i64).abs() <= 1, "{rate}: {}", buf.len());
        }
    }

    #[test]
    fn audio_mix_like_vice() {
        assert_eq!(audio_mix(0, 123), 123);
        assert_eq!(audio_mix(1000, -300), 700);
        assert_eq!(audio_mix(30000, 30000), 32535);
        assert_eq!(audio_mix(-30000, -30000), -32535);
    }

    #[test]
    fn no_samples_without_audio() {
        let mut sid = Sid::new();
        let mut buf = Vec::new();
        run(&mut sid, 20_000);
        sid.take_samples(&mut buf);
        assert!(buf.is_empty());
    }

    #[test]
    fn osc3_follows_oscillator_within_frame() {
        let mut sid = Sid::new();
        sid.write(0x0E, 0x00);
        sid.write(0x0F, 0x10); // +$1000 per cycle: OSC3 advances by 1 every 16 cycles
        sid.write(0x12, 0x20); // sawtooth
        let a = sid.read_mut(0x1B);
        run(&mut sid, 160);
        let b = sid.read_mut(0x1B);
        assert_eq!(b.wrapping_sub(a), 10);
    }

    #[test]
    fn noise_matches_resid_sequence() {
        // First noise values after power-on (LFSR $7FFFFE), as in VICE
        let mut sid = Sid::new();
        sid.write(0x0E, 0xFF);
        sid.write(0x0F, 0xFF);
        sid.write(0x12, 0x80);
        let mut seq = vec![sid.read_mut(0x1B)];
        while seq.len() < 6 {
            sid.tick();
            let v = sid.read_mut(0x1B);
            if v != *seq.last().unwrap() { seq.push(v); }
        }
        assert_eq!(seq, [0xFE, 0xFC, 0xF8, 0xF0, 0xE0, 0xC0]);
    }

    #[test]
    fn write_only_registers_read_the_data_bus() {
        let mut sid = Sid::new();
        sid.write(0x05, 0x5A);
        assert_eq!(sid.read_mut(0x00), 0x5A);
        run(&mut sid, 0x1D00 - 1);
        assert_eq!(sid.read_mut(0x12), 0x5A);
        run(&mut sid, 1);
        assert_eq!(sid.read_mut(0x12), 0x00);
    }

    /// Audio output peak over 0.2 s of a sawtooth voice with
    /// frequency `freq`, with `filt` in $D417 and `mode_vol` in $D418.
    fn peak(model: Model, freq: u16, filt: u8, mode_vol: u8, cutoff: u16) -> i32 {
        let mut sid = Sid::new();
        sid.set_model(model, false);
        sid.set_audio(Some(44_100)).unwrap();
        sid.write(0x15, (cutoff & 7) as u8);
        sid.write(0x16, (cutoff >> 3) as u8);
        sid.write(0x17, filt);
        sid.write(0x18, mode_vol);
        sid.write(0x00, freq as u8);
        sid.write(0x01, (freq >> 8) as u8);
        sid.write(0x05, 0x00);
        sid.write(0x06, 0xF0);
        sid.write(0x04, 0x21);
        let mut buf = Vec::new();
        run(&mut sid, 200_000);
        sid.take_samples(&mut buf);
        let tail = &buf[buf.len() / 2..];
        let mean = tail.iter().map(|&s| s as i64).sum::<i64>() / tail.len() as i64;
        tail.iter().map(|&s| (s as i64 - mean).abs() as i32).max().unwrap()
    }

    #[test]
    fn filter_lowpass_attenuates_high_notes() {
        for model in [Model::Mos6581, Model::Mos8580] {
            // Low (~110 Hz) and high (~3.5 kHz) note with a low-cutoff low-pass
            // Cutoff around 300 Hz: the 8580 is nearly linear, the 6581 is not
            let cutoff = if model == Model::Mos6581 { 0x100 } else { 0x40 };
            let low = peak(model, 0x0730, 0x01, 0x1F, cutoff);
            let high = peak(model, 0xE600, 0x01, 0x1F, cutoff);
            let direct = peak(model, 0xE600, 0x00, 0x0F, cutoff);
            assert!(low > 1000, "{model:?}: {low}");
            assert!(high * 4 < direct, "{model:?}: {high} vs {direct}");
        }
    }

    #[test]
    fn audio_resumes_without_click() {
        // Sawtooth, audio off for half a second (turbo) and back on:
        // restarts from zero, with no jump and no DC component
        let mut sid = Sid::new();
        sid.set_audio(Some(48_000)).unwrap();
        for (r, v) in [(0x18, 0x0F), (0x05, 0x00), (0x06, 0xF0), (0x01, 0x10), (0x04, 0x21)] {
            sid.write(r, v);
        }
        let mut buf = Vec::new();
        run(&mut sid, 200_000);
        sid.take_samples(&mut buf);
        sid.set_audio(None).unwrap();
        run(&mut sid, 500_000);
        sid.set_audio(Some(48_000)).unwrap();
        buf.clear();
        run(&mut sid, 200_000);
        sid.take_samples(&mut buf);
        assert!(buf[0].abs() < 100, "{}", buf[0]);
        let mean = |s: &[i16]| s.iter().map(|&x| x as i64).sum::<i64>() / s.len() as i64;
        assert!(mean(&buf[..1920]).abs() < 300, "{}", mean(&buf[..1920]));
    }

    #[test]
    fn first_audio_has_no_power_on_transient() {
        let mut sid = Sid::new();
        run(&mut sid, 50_000);
        sid.set_audio(Some(44_100)).unwrap();
        let mut buf = Vec::new();
        run(&mut sid, 50_000);
        sid.take_samples(&mut buf);
        assert!(buf.iter().all(|s| s.abs() < 100), "{:?}", buf.iter().min());
    }

    #[test]
    fn volume_register_makes_digis_audible_on_6581() {
        // No voice: the volume only shifts the DC component. With the
        // volume changing at 4 kHz the output oscillates on the 6581.
        let swing = |model: Model| {
            let mut sid = Sid::new();
            sid.set_model(model, false);
            sid.set_audio(Some(44_100)).unwrap();
            let mut buf = Vec::new();
            for k in 0..800 {
                sid.write(0x18, if k & 1 == 0 { 0x0F } else { 0x00 });
                run(&mut sid, 123);
            }
            sid.take_samples(&mut buf);
            let tail = &buf[buf.len() / 2..];
            tail.iter().max().unwrap() - tail.iter().min().unwrap()
        };
        assert!(swing(Model::Mos6581) > 2000);
    }
}

