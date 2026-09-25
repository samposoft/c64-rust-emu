// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! C64 audio output stage (reSID 1.0, extfilt.h): after the SID, a 16 kHz
//! RC low-pass (10 kΩ, 1000 pF) and a 16 Hz high-pass (1 kΩ, 10 µF) that
//! removes the DC component. 27-bit integer arithmetic, clocked at 1 MHz.

pub struct ExternalFilter {
    vlp: i32,
    vhp: i32,
}

// w0 = dt/(dt + RC) with dt = 1 µs, in fixed point
const W0LP_1_S7: i32 = (1e-6 / (1e-6 + 1e4 * 1e-9) * (1 << 7) as f64 + 0.5) as i32;
const W0HP_1_S17: i32 = (1e-6 / (1e-6 + 1e3 * 1e-5) * (1 << 17) as f64 + 0.5) as i32;

impl ExternalFilter {
    pub fn new() -> Self {
        Self { vlp: 0, vhp: 0 }
    }

    pub fn reset(&mut self) {
        self.vlp = 0;
        self.vhp = 0;
    }

    /// The low-pass takes the value `vi` at once; the high-pass, i.e. the
    /// average signal level, stays as it is, or starts from `vi` if the stage
    /// has never run (when audio is switched on): output 0, without the
    /// DC transient.
    pub fn resume(&mut self, vi: i16) {
        if self.vlp == 0 && self.vhp == 0 {
            self.vhp = (vi as i32) << 11;
        }
        self.vlp = (vi as i32) << 11;
    }

    #[inline]
    pub fn clock(&mut self, vi: i16) {
        let dvlp = W0LP_1_S7.wrapping_mul(((vi as i32 as u32) << 11).wrapping_sub(self.vlp as u32) as i32) >> 7;
        let dvhp = W0HP_1_S17.wrapping_mul(self.vlp.wrapping_sub(self.vhp)) >> 17;
        self.vlp = self.vlp.wrapping_add(dvlp);
        self.vhp = self.vhp.wrapping_add(dvhp);
    }

    /// 16-bit output.
    #[inline]
    pub fn output(&self) -> i32 {
        self.vlp.wrapping_sub(self.vhp) >> 11
    }
}

use crate::snapshot::impl_state;

impl_state!(ExternalFilter { vlp, vhp });
