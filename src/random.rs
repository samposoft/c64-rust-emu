// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 (lib.c), Copyright (C) Andreas Boose, Marco van
// den Heuvel.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! VICE's pseudo-random generator (lib.c): PCG-XSH-RR 32/64, a 64-bit
//! linear congruential generator with a 32-bit permutation output. VICE
//! seeds it from the clock at startup; here it always starts from the
//! state VICE has before seeding, so that two runs are identical (same
//! power-on RAM, same tape errors).

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const INCREMENT: u64 = 1;

#[derive(Clone)]
pub struct Pcg {
    state: u64,
}

impl Pcg {
    pub fn new() -> Pcg {
        Pcg { state: MULTIPLIER.wrapping_add(INCREMENT) }
    }

    /// 32-bit random number (VICE's rand_uint32).
    pub fn next_u32(&mut self) -> u32 {
        let prev = self.state;
        self.state = prev.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
        let base = ((prev ^ (prev >> 18)) >> 27) as u32;
        base.rotate_right((prev >> 59) as u32)
    }

    /// Uniform in [0, 1) (VICE's lib_double_rand_unit).
    pub fn unit(&mut self) -> f64 {
        self.next_u32() as f64 / 4_294_967_296.0
    }
}

impl Default for Pcg {
    fn default() -> Self {
        Self::new()
    }
}

crate::snapshot::impl_state!(Pcg { state });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_sequence_as_vice() {
        // First values of VICE's rand_uint32 (compiled lib.c) from the
        // initial state, without a seed
        let mut r = Pcg::new();
        let got: Vec<u32> = (0..4).map(|_| r.next_u32()).collect();
        assert_eq!(got, [0xE4C14788, 0x379C6516, 0x5C4AB3BB, 0x601D23E0]);
        assert!((0..1000).map(|_| r.unit()).all(|u| (0.0..1.0).contains(&u)));
    }
}
