// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! SID D/A converters (reSID 1.0, dac.cc): R-2R ladders with the
//! imperfections of real chips, computed at compile time.
//!
//! The 6581 lacks the termination resistor at bit 0 and has poorly matched
//! R and 2R (2R/R ≈ 2.20): the low bits weigh almost as much as their
//! neighbours and the output is not monotonic. The 8580 has the termination
//! and 2R/R = 2.00. In both, a transistor that is off leaks a small current.

const MOSFET_LEAKAGE_6581: f64 = 0.0075;
const MOSFET_LEAKAGE_8580: f64 = 0.0035;

/// Table of a `bits`-bit DAC (at most 12): for each digital value, the
/// output voltage scaled to the range 0..2^bits - 1.
pub const fn build_dac_table<const N: usize>(bits: usize, r2_div_r: f64, term: bool) -> [u16; N] {
    let mut vbit = [0.0f64; 12];
    let leakage = if term { MOSFET_LEAKAGE_8580 } else { MOSFET_LEAKAGE_6581 };

    // Contribution of each bit of the R-2R ladder
    let mut set_bit = 0;
    while set_bit < bits {
        let mut vn = 1.0; // normalized bit voltage
        let r = 1.0;
        let r2 = r2_div_r * r;
        // Resistance of the "tail": 2R with termination, infinite without
        let mut rn = if term { r2 } else { f64::INFINITY };

        let mut bit = 0;
        while bit < set_bit {
            if rn == f64::INFINITY {
                rn = r + r2;
            } else {
                rn = r + r2 * rn / (r2 + rn); // R + 2R || Rn
            }
            bit += 1;
        }

        // Source transformation of the bit
        if rn == f64::INFINITY {
            rn = r2;
        } else {
            rn = r2 * rn / (r2 + rn); // 2R || Rn
            vn = vn * rn / r2;
        }

        // Output voltage by successive transformations from the tail
        bit += 1;
        while bit < bits {
            rn += r;
            let i = vn / rn;
            rn = r2 * rn / (r2 + rn); // 2R || Rn
            vn = rn * i;
            bit += 1;
        }
        vbit[set_bit] = vn;
        set_bit += 1;
    }

    // Any bit combination by superposition
    let mut dac = [0u16; N];
    let mut i = 0;
    while i < (1 << bits) {
        let mut x = i;
        let mut vo = 0.0;
        let mut j = 0;
        while j < bits {
            vo += (if x & 1 != 0 { 1.0 } else { leakage }) * vbit[j];
            x >>= 1;
            j += 1;
        }
        // Maximum output scaled to 2^bits - 1
        dac[i] = (((1 << bits) - 1) as f64 * vo + 0.5) as u16;
        i += 1;
    }
    dac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dac_8580_is_nearly_linear_6581_is_not() {
        let d8580: [u16; 4096] = build_dac_table(12, 2.00, true);
        let d6581: [u16; 4096] = build_dac_table(12, 2.20, false);
        // 8580: monotonic
        assert!(d8580.windows(2).all(|w| w[1] >= w[0]));
        // 6581: without termination bit 0 weighs almost as much as bit 1, and
        // the ladder is not monotonic where the high bits change
        assert!(d6581[2] - d6581[1] <= 1);
        assert!(d6581.windows(2).any(|w| w[1] < w[0]));
    }
}
