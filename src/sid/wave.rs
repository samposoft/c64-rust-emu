// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! Waveform generator (reSID 1.0 as shipped in VICE 3.10,
//! wave.h/wave.cc), clocked one cycle at a time.
//!
//! A 24-bit accumulator advances by FREQ every cycle; triangle, sawtooth
//! and pulse use its top 12 bits, noise comes from a 23-bit shift register
//! clocked by bit 19. Combined waveforms are not an AND: the bits short
//! each other and the output is looked up in tables sampled from OSC3 on
//! real chips (6581 and 8580). Combinations with noise write the cleared
//! bits back into the shift register, and on the 6581 the combined
//! sawtooth can pull down the accumulator MSB.

use super::dac::build_dac_table;
use super::Model;

// Cycles after which, with the test bit high, the shift register fills
// with ones (the SRAM cells slowly rise towards 1)
const SHIFT_REGISTER_RESET_START_6581: i32 = 35_000;
const SHIFT_REGISTER_RESET_BIT_6581: i32 = 1_000;
const SHIFT_REGISTER_RESET_START_8580: i32 = 2_519_864;
const SHIFT_REGISTER_RESET_BIT_8580: i32 = 315_000;

// Cycles after which, with no waveform selected, the floating DAC input
// loses its bits one at a time
const FLOATING_OUTPUT_TTL_START_6581: i32 = 182_000;
const FLOATING_OUTPUT_TTL_BIT_6581: i32 = 1_500;
const FLOATING_OUTPUT_TTL_START_8580: i32 = 4_400_000;
const FLOATING_OUTPUT_TTL_BIT_8580: i32 = 50_000;

/// Waveform output (without noise) per model and bits 4-6 of the
/// control register, indexed by the top 12 bits of the accumulator.
/// The combinations come from reSID's OSC3 samples (8 bits, << 4).
static MODEL_WAVE: [[[u16; 4096]; 8]; 2] = [
    build_waves([
        include_bytes!("samples/wave6581__ST.dat"),
        include_bytes!("samples/wave6581_P_T.dat"),
        include_bytes!("samples/wave6581_PS_.dat"),
        include_bytes!("samples/wave6581_PST.dat"),
    ]),
    build_waves([
        include_bytes!("samples/wave8580__ST.dat"),
        include_bytes!("samples/wave8580_P_T.dat"),
        include_bytes!("samples/wave8580_PS_.dat"),
        include_bytes!("samples/wave8580_PST.dat"),
    ]),
];

/// 12-bit waveform DAC (6581: 2R/R 2.20 without termination;
/// 8580: 2.00 with termination).
static MODEL_DAC: [[u16; 4096]; 2] = [
    build_dac_table(12, 2.20, false),
    build_dac_table(12, 2.00, true),
];

/// `st`, `p_t`, `ps_`, `pst`: samples of combinations 3, 5, 6 and 7.
const fn build_waves(samples: [&[u8; 4096]; 4]) -> [[u16; 4096]; 8] {
    let mut w = [[0u16; 4096]; 8];
    let mut i = 0;
    let mut acc: u32 = 0;
    while i < 4096 {
        let msb = acc & 0x800000;
        // Noise mask, triangle, sawtooth, pulse mask
        w[0][i] = 0xFFF;
        w[1][i] = (((acc ^ if msb != 0 { 0xFFFF_FFFF } else { 0 }) >> 11) & 0xFFE) as u16;
        w[2][i] = (acc >> 12) as u16;
        w[4][i] = 0xFFF;
        w[3][i] = (samples[0][i] as u16) << 4;
        w[5][i] = (samples[1][i] as u16) << 4;
        w[6][i] = (samples[2][i] as u16) << 4;
        w[7][i] = (samples[3][i] as u16) << 4;
        acc += 0x1000;
        i += 1;
    }
    w
}

/// Noise combined with pulse: the bits switch each other off.
fn noise_pulse6581(noise: u16) -> u16 {
    if noise < 0xF00 { 0 } else { noise & (noise << 1) & (noise << 2) }
}

fn noise_pulse8580(noise: u16) -> u16 {
    if noise < 0xFC0 { noise & (noise << 1) } else { 0xFC0 }
}

/// Write to the shift register on the falling edge of the test bit,
/// if the previous waveform was combined with noise.
fn do_pre_writeback(waveform_prev: u8, waveform: u8, is6581: bool) -> bool {
    // No write without combined waveforms
    if waveform_prev <= 0x8 {
        return false;
    }
    if waveform_prev == 0xC {
        if is6581 {
            return false;
        } else if waveform != 0x9 && waveform != 0xE {
            return false;
        }
    }
    if is6581
        && ((waveform_prev & 0x3 == 0x1 && waveform & 0x3 == 0x2)
            || (waveform_prev & 0x3 == 0x2 && waveform & 0x3 == 0x1))
    {
        return false;
    }
    true
}

pub struct WaveformGenerator {
    model: Model,
    pub accumulator: u32,
    /// The accumulator MSB rose in this cycle (sync source).
    pub msb_rising: bool,
    pub freq: u32,
    pw: u32,
    shift_register: u32,
    /// Cycles left before the shift register fills with ones.
    shift_register_reset: i32,
    /// The shift happens two cycles after the bit 19 edge.
    shift_pipeline: i32,
    ring_msb_mask: u32,
    no_noise: u16,
    noise_output: u16,
    no_noise_or_noise_output: u16,
    no_pulse: u16,
    /// Pulse level: computed at the end of a cycle, used in the next one.
    pulse_output: u16,
    /// Bits 4-7 of the control register.
    pub waveform: u8,
    /// 8580: triangle and sawtooth reach OSC3 one cycle later.
    tri_saw_pipeline: u16,
    osc3: u16,
    test: bool,
    ring_mod: bool,
    pub sync: bool,
    /// DAC input.
    waveform_output: u16,
    /// Cycles before the floating input (no waveform) loses a bit.
    floating_output_ttl: i32,
}

impl WaveformGenerator {
    pub fn new() -> Self {
        let mut w = Self {
            model: Model::Mos6581,
            // At power-on the even bits of the accumulator are high
            accumulator: 0x555555,
            msb_rising: false,
            freq: 0,
            pw: 0,
            shift_register: 0,
            shift_register_reset: 0,
            shift_pipeline: 0,
            ring_msb_mask: 0,
            no_noise: 0,
            noise_output: 0,
            no_noise_or_noise_output: 0,
            no_pulse: 0,
            pulse_output: 0,
            waveform: 0,
            tri_saw_pipeline: 0x555,
            osc3: 0,
            test: false,
            ring_mod: false,
            sync: false,
            waveform_output: 0,
            floating_output_ttl: 0,
        };
        w.reset();
        w
    }

    pub fn set_model(&mut self, model: Model) {
        self.model = model;
    }

    /// Chip reset: the accumulator does not change; when reset is released
    /// the shift register advances once.
    pub fn reset(&mut self) {
        self.freq = 0;
        self.pw = 0;
        self.msb_rising = false;
        self.waveform = 0;
        self.test = false;
        self.ring_mod = false;
        self.sync = false;
        self.ring_msb_mask = 0;
        self.no_noise = 0xFFF;
        self.no_pulse = 0xFFF;
        self.pulse_output = 0xFFF;
        self.shift_register = 0x7FFFFE;
        self.shift_register_reset = 0;
        self.set_noise_output();
        self.shift_pipeline = 0;
        self.waveform_output = 0;
        self.osc3 = 0;
        self.floating_output_ttl = 0;
    }

    fn is6581(&self) -> bool {
        self.model == Model::Mos6581
    }

    fn wave(&self) -> &'static [u16; 4096] {
        &MODEL_WAVE[self.model as usize][(self.waveform & 0x7) as usize]
    }

    // ── Registers ────────────────────────────────────────────────────────────

    pub fn write_freq_lo(&mut self, v: u8) {
        self.freq = (self.freq & 0xFF00) | v as u32;
    }

    pub fn write_freq_hi(&mut self, v: u8) {
        self.freq = ((v as u32) << 8) | (self.freq & 0x00FF);
    }

    pub fn write_pw_lo(&mut self, v: u8) {
        self.pw = (self.pw & 0xF00) | v as u32;
        self.pulse_output = if (self.accumulator >> 12) >= self.pw { 0xFFF } else { 0 };
    }

    pub fn write_pw_hi(&mut self, v: u8) {
        self.pw = (((v as u32) << 8) & 0xF00) | (self.pw & 0x0FF);
        self.pulse_output = if (self.accumulator >> 12) >= self.pw { 0xFFF } else { 0 };
    }

    /// Control register. `sync_source_acc`: accumulator of the voice that
    /// modulates this one (ring modulation).
    pub fn write_control(&mut self, control: u8, sync_source_acc: u32) {
        let waveform_prev = self.waveform;
        let test_prev = self.test;
        self.waveform = (control >> 4) & 0x0F;
        self.test = control & 0x08 != 0;
        self.ring_mod = control & 0x04 != 0;
        self.sync = control & 0x02 != 0;

        // The accumulator MSB is replaced by ring modulation when sawtooth
        // is off
        self.ring_msb_mask = (((!control >> 5) & (control >> 2) & 0x1) as u32) << 23;
        self.no_noise = if self.waveform & 0x8 != 0 { 0 } else { 0xFFF };
        self.no_noise_or_noise_output = self.no_noise | self.noise_output;
        self.no_pulse = if self.waveform & 0x4 != 0 { 0 } else { 0xFFF };

        if !test_prev && self.test {
            // Test rising: accumulator cleared, pending shift cancelled,
            // the shift register cells start rising towards 1
            self.accumulator = 0;
            self.shift_pipeline = 0;
            self.shift_register_reset = if self.is6581() {
                SHIFT_REGISTER_RESET_START_6581
            } else {
                SHIFT_REGISTER_RESET_START_8580
            };
            self.pulse_output = 0xFFF;
        } else if test_prev && !self.test {
            // Test falling: the second phase of the shift completes, in
            // which the output may overwrite the latched value
            if do_pre_writeback(waveform_prev, self.waveform, self.is6581()) {
                self.write_shift_register();
            }
            // bit0 = (bit22 | test) ^ bit17 = ~bit17
            let bit0 = (!self.shift_register >> 17) & 0x1;
            self.shift_register = ((self.shift_register << 1) | bit0) & 0x7FFFFF;
            self.set_noise_output();
        }

        if self.waveform != 0 {
            self.set_waveform_output(sync_source_acc);
        } else if waveform_prev != 0 {
            // Floating DAC input: starts fading
            self.floating_output_ttl = if self.is6581() {
                FLOATING_OUTPUT_TTL_START_6581
            } else {
                FLOATING_OUTPUT_TTL_START_8580
            };
        }
    }

    /// $D41B for voice 3.
    pub fn read_osc(&self) -> u8 {
        (self.osc3 >> 4) as u8
    }

    // ── Clock ────────────────────────────────────────────────────────────────

    /// One cycle: accumulator and shift register.
    #[inline]
    pub fn clock(&mut self) {
        if self.test {
            if self.shift_register_reset != 0 {
                self.shift_register_reset -= 1;
                if self.shift_register_reset == 0 {
                    self.shiftreg_bitfade();
                }
            }
            // The test bit holds the pulse high
            self.pulse_output = 0xFFF;
        } else {
            let accumulator_next = (self.accumulator + self.freq) & 0xFFFFFF;
            let bits_set = !self.accumulator & accumulator_next;
            self.accumulator = accumulator_next;
            self.msb_rising = bits_set & 0x800000 != 0;

            // The shift register advances two cycles after the rising edge
            // of bit 19
            if bits_set & 0x080000 != 0 {
                self.shift_pipeline = 2;
            } else if self.shift_pipeline != 0 {
                self.shift_pipeline -= 1;
                if self.shift_pipeline == 0 {
                    self.clock_shift_register();
                }
            }
        }
    }

    fn clock_shift_register(&mut self) {
        // bit0 = (bit22 | test) ^ bit17
        let bit0 = ((self.shift_register >> 22) ^ (self.shift_register >> 17)) & 0x1;
        self.shift_register = ((self.shift_register << 1) | bit0) & 0x7FFFFF;
        self.set_noise_output();
    }

    /// Combinations with noise write the cleared output bits back into the
    /// shift register (a bit at zero does not return to one).
    fn write_shift_register(&mut self) {
        let wo = self.waveform_output as u32;
        self.shift_register &= !((1 << 20) | (1 << 18) | (1 << 14) | (1 << 11) | (1 << 9) | (1 << 5) | (1 << 2) | 1)
            | ((wo & 0x800) << 9)   // bit 11 -> bit 20
            | ((wo & 0x400) << 8)   // bit 10 -> bit 18
            | ((wo & 0x200) << 5)   // bit  9 -> bit 14
            | ((wo & 0x100) << 3)   // bit  8 -> bit 11
            | ((wo & 0x080) << 2)   // bit  7 -> bit  9
            | ((wo & 0x040) >> 1)   // bit  6 -> bit  5
            | ((wo & 0x020) >> 3)   // bit  5 -> bit  2
            | ((wo & 0x010) >> 4);  // bit  4 -> bit  0
        self.noise_output &= self.waveform_output;
        self.no_noise_or_noise_output = self.no_noise | self.noise_output;
    }

    fn set_noise_output(&mut self) {
        let s = self.shift_register;
        self.noise_output = (((s & 0x100000) >> 9)
            | ((s & 0x040000) >> 8)
            | ((s & 0x004000) >> 5)
            | ((s & 0x000800) >> 3)
            | ((s & 0x000200) >> 2)
            | ((s & 0x000020) << 1)
            | ((s & 0x000004) << 3)
            | ((s & 0x000001) << 4)) as u16;
        self.no_noise_or_noise_output = self.no_noise | self.noise_output;
    }

    /// Hard sync, after all voices have been clocked: `dest` is the voice
    /// synced by this one, `source` the one that syncs this one. If the
    /// source is itself synced in the cycle in which its MSB rises, the
    /// destination is not reset (verified on OSC3).
    #[inline]
    pub fn sync_dest(&self, dest_sync: bool, source_msb_rising: bool) -> bool {
        self.msb_rising && dest_sync && !(self.sync && source_msb_rising)
    }

    /// Waveform output for this cycle and pulse level for the
    /// next one.
    #[inline]
    pub fn set_waveform_output(&mut self, sync_source_acc: u32) {
        if self.waveform != 0 {
            let ix = ((self.accumulator ^ (!sync_source_acc & self.ring_msb_mask)) >> 12) as usize;
            let wave = self.wave();
            self.waveform_output = wave[ix] & (self.no_pulse | self.pulse_output) & self.no_noise_or_noise_output;
            if self.waveform & 0xC == 0xC {
                self.waveform_output = if self.is6581() {
                    noise_pulse6581(self.waveform_output)
                } else {
                    noise_pulse8580(self.waveform_output)
                };
            }
            // 8580: triangle and sawtooth come out half a cycle later, i.e. one
            // cycle later on OSC3, which is read in the first clock phase
            if self.waveform & 3 != 0 && self.model == Model::Mos8580 {
                self.osc3 = self.tri_saw_pipeline & (self.no_pulse | self.pulse_output) & self.no_noise_or_noise_output;
                self.tri_saw_pipeline = wave[ix];
            } else {
                self.osc3 = self.waveform_output;
            }
            // 6581: with combined sawtooth the accumulator MSB can be
            // pulled low
            if self.waveform & 0x2 != 0 && self.waveform & 0xD != 0 && self.is6581() {
                self.accumulator &= ((self.waveform_output as u32) << 12) | 0x7FFFFF;
            }
            if self.waveform > 0x8 && !self.test && self.shift_pipeline != 1 {
                self.write_shift_register();
            }
        } else if self.floating_output_ttl != 0 {
            self.floating_output_ttl -= 1;
            if self.floating_output_ttl == 0 {
                self.wave_bitfade();
            }
        }
        // The pulse comparison arrives one cycle late
        self.pulse_output = if (self.accumulator >> 12) >= self.pw { 0xFFF } else { 0 };
    }

    fn wave_bitfade(&mut self) {
        self.waveform_output &= self.waveform_output >> 1;
        self.osc3 = self.waveform_output;
        if self.waveform_output != 0 {
            self.floating_output_ttl = if self.is6581() {
                FLOATING_OUTPUT_TTL_BIT_6581
            } else {
                FLOATING_OUTPUT_TTL_BIT_8580
            };
        }
    }

    fn shiftreg_bitfade(&mut self) {
        self.shift_register |= 1;
        self.shift_register |= self.shift_register << 1;
        self.shift_register &= 0x7FFFFF;
        self.set_noise_output();
        if self.shift_register != 0x7FFFFF {
            self.shift_register_reset = if self.is6581() {
                SHIFT_REGISTER_RESET_BIT_6581
            } else {
                SHIFT_REGISTER_RESET_BIT_8580
            };
        }
    }

    /// 12-bit DAC output, with the model's imperfections.
    #[inline]
    pub fn output(&self) -> i32 {
        MODEL_DAC[self.model as usize][self.waveform_output as usize] as i32
    }
}

use crate::snapshot::impl_state;

impl_state!(WaveformGenerator {
    model, accumulator, msb_rising, freq, pw, shift_register, shift_register_reset,
    shift_pipeline, ring_msb_mask, no_noise, noise_output, no_noise_or_noise_output, no_pulse,
    pulse_output, waveform, tri_saw_pipeline, osc3, test, ring_mod, sync, waveform_output,
    floating_output_ttl,
});
