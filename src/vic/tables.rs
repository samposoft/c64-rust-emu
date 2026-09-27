// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 (src/viciisc/vicii-chip-model.c), Copyright (C)
// Hannu Nuotio, Daniel Kahlin.
// Converted to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! Cycle tables of the VIC-II models, converted from VICE's `cycle_tab_pal`
//! (6569) and `cycle_tab_ntsc` (6567R8) as `vicii_chip_model_set` combines
//! their phi1 and phi2 entries. Index: our cycle k = cycle % cycles per line
//! (k = 0 is the last cycle of the line). xpos is the phi1 X rounded to 8.

use super::{Cyc, Phi1};
use super::{BRD_L0, BRD_L1, BRD_R0, BRD_R1, FETCH_BA, SPR_CRUNCH, SPR_DISP, SPR_DMA, SPR_EXP};
use super::{UPDATE_MCBASE, UPDATE_RC, UPDATE_VC, VISIBLE};

pub(super) const PAL: [Cyc; 63] = [
    Cyc { flags: 0, phi1: Phi1::SprDma1(2), xpos: 0x188, spr_ba: 0b00011100 }, // 63
    Cyc { flags: 0, phi1: Phi1::SprPtr(3), xpos: 0x190, spr_ba: 0b00011000 }, // 1
    Cyc { flags: 0, phi1: Phi1::SprDma1(3), xpos: 0x198, spr_ba: 0b00111000 }, // 2
    Cyc { flags: 0, phi1: Phi1::SprPtr(4), xpos: 0x1a0, spr_ba: 0b00110000 }, // 3
    Cyc { flags: 0, phi1: Phi1::SprDma1(4), xpos: 0x1a8, spr_ba: 0b01110000 }, // 4
    Cyc { flags: 0, phi1: Phi1::SprPtr(5), xpos: 0x1b0, spr_ba: 0b01100000 }, // 5
    Cyc { flags: 0, phi1: Phi1::SprDma1(5), xpos: 0x1b8, spr_ba: 0b11100000 }, // 6
    Cyc { flags: 0, phi1: Phi1::SprPtr(6), xpos: 0x1c0, spr_ba: 0b11000000 }, // 7
    Cyc { flags: 0, phi1: Phi1::SprDma1(6), xpos: 0x1c8, spr_ba: 0b11000000 }, // 8
    Cyc { flags: 0, phi1: Phi1::SprPtr(7), xpos: 0x1d0, spr_ba: 0b10000000 }, // 9
    Cyc { flags: 0, phi1: Phi1::SprDma1(7), xpos: 0x1d8, spr_ba: 0b10000000 }, // 10
    Cyc { flags: 0, phi1: Phi1::Refresh, xpos: 0x1e0, spr_ba: 0b00000000 }, // 11
    Cyc { flags: FETCH_BA, phi1: Phi1::Refresh, xpos: 0x1e8, spr_ba: 0b00000000 }, // 12
    Cyc { flags: FETCH_BA, phi1: Phi1::Refresh, xpos: 0x1f0, spr_ba: 0b00000000 }, // 13
    Cyc { flags: FETCH_BA|UPDATE_VC, phi1: Phi1::Refresh, xpos: 0x000, spr_ba: 0b00000000 }, // 14
    Cyc { flags: VISIBLE|FETCH_BA|SPR_CRUNCH, phi1: Phi1::Refresh, xpos: 0x008, spr_ba: 0b00000000 }, // 15
    Cyc { flags: VISIBLE|FETCH_BA|UPDATE_MCBASE, phi1: Phi1::FetchG, xpos: 0x010, spr_ba: 0b00000000 }, // 16
    Cyc { flags: VISIBLE|FETCH_BA|BRD_L1, phi1: Phi1::FetchG, xpos: 0x018, spr_ba: 0b00000000 }, // 17
    Cyc { flags: VISIBLE|FETCH_BA|BRD_L0, phi1: Phi1::FetchG, xpos: 0x020, spr_ba: 0b00000000 }, // 18
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x028, spr_ba: 0b00000000 }, // 19
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x030, spr_ba: 0b00000000 }, // 20
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x038, spr_ba: 0b00000000 }, // 21
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x040, spr_ba: 0b00000000 }, // 22
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x048, spr_ba: 0b00000000 }, // 23
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x050, spr_ba: 0b00000000 }, // 24
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x058, spr_ba: 0b00000000 }, // 25
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x060, spr_ba: 0b00000000 }, // 26
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x068, spr_ba: 0b00000000 }, // 27
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x070, spr_ba: 0b00000000 }, // 28
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x078, spr_ba: 0b00000000 }, // 29
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x080, spr_ba: 0b00000000 }, // 30
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x088, spr_ba: 0b00000000 }, // 31
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x090, spr_ba: 0b00000000 }, // 32
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x098, spr_ba: 0b00000000 }, // 33
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0a0, spr_ba: 0b00000000 }, // 34
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0a8, spr_ba: 0b00000000 }, // 35
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0b0, spr_ba: 0b00000000 }, // 36
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0b8, spr_ba: 0b00000000 }, // 37
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0c0, spr_ba: 0b00000000 }, // 38
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0c8, spr_ba: 0b00000000 }, // 39
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0d0, spr_ba: 0b00000000 }, // 40
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0d8, spr_ba: 0b00000000 }, // 41
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0e0, spr_ba: 0b00000000 }, // 42
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0e8, spr_ba: 0b00000000 }, // 43
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0f0, spr_ba: 0b00000000 }, // 44
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0f8, spr_ba: 0b00000000 }, // 45
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x100, spr_ba: 0b00000000 }, // 46
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x108, spr_ba: 0b00000000 }, // 47
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x110, spr_ba: 0b00000000 }, // 48
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x118, spr_ba: 0b00000000 }, // 49
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x120, spr_ba: 0b00000000 }, // 50
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x128, spr_ba: 0b00000000 }, // 51
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x130, spr_ba: 0b00000000 }, // 52
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x138, spr_ba: 0b00000000 }, // 53
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x140, spr_ba: 0b00000000 }, // 54
    Cyc { flags: SPR_DMA, phi1: Phi1::FetchG, xpos: 0x148, spr_ba: 0b00000001 }, // 55
    Cyc { flags: SPR_DMA|SPR_EXP|BRD_R0, phi1: Phi1::Idle, xpos: 0x150, spr_ba: 0b00000001 }, // 56
    Cyc { flags: BRD_R1, phi1: Phi1::Idle, xpos: 0x158, spr_ba: 0b00000011 }, // 57
    Cyc { flags: UPDATE_RC|SPR_DISP, phi1: Phi1::SprPtr(0), xpos: 0x160, spr_ba: 0b00000011 }, // 58
    Cyc { flags: 0, phi1: Phi1::SprDma1(0), xpos: 0x168, spr_ba: 0b00000111 }, // 59
    Cyc { flags: 0, phi1: Phi1::SprPtr(1), xpos: 0x170, spr_ba: 0b00000110 }, // 60
    Cyc { flags: 0, phi1: Phi1::SprDma1(1), xpos: 0x178, spr_ba: 0b00001110 }, // 61
    Cyc { flags: 0, phi1: Phi1::SprPtr(2), xpos: 0x180, spr_ba: 0b00001100 }, // 62
];

pub(super) const NTSC: [Cyc; 65] = [
    Cyc { flags: 0, phi1: Phi1::SprPtr(3), xpos: 0x190, spr_ba: 0b00011000 }, // 65
    Cyc { flags: 0, phi1: Phi1::SprDma1(3), xpos: 0x198, spr_ba: 0b00111000 }, // 1
    Cyc { flags: 0, phi1: Phi1::SprPtr(4), xpos: 0x1a0, spr_ba: 0b00110000 }, // 2
    Cyc { flags: 0, phi1: Phi1::SprDma1(4), xpos: 0x1a8, spr_ba: 0b01110000 }, // 3
    Cyc { flags: 0, phi1: Phi1::SprPtr(5), xpos: 0x1b0, spr_ba: 0b01100000 }, // 4
    Cyc { flags: 0, phi1: Phi1::SprDma1(5), xpos: 0x1b8, spr_ba: 0b11100000 }, // 5
    Cyc { flags: 0, phi1: Phi1::SprPtr(6), xpos: 0x1c0, spr_ba: 0b11000000 }, // 6
    Cyc { flags: 0, phi1: Phi1::SprDma1(6), xpos: 0x1c8, spr_ba: 0b11000000 }, // 7
    Cyc { flags: 0, phi1: Phi1::SprPtr(7), xpos: 0x1d0, spr_ba: 0b10000000 }, // 8
    Cyc { flags: 0, phi1: Phi1::SprDma1(7), xpos: 0x1d8, spr_ba: 0b10000000 }, // 9
    Cyc { flags: 0, phi1: Phi1::Idle, xpos: 0x1e0, spr_ba: 0b00000000 }, // 10
    Cyc { flags: 0, phi1: Phi1::Refresh, xpos: 0x1e8, spr_ba: 0b00000000 }, // 11
    Cyc { flags: FETCH_BA, phi1: Phi1::Refresh, xpos: 0x1f0, spr_ba: 0b00000000 }, // 12
    Cyc { flags: FETCH_BA, phi1: Phi1::Refresh, xpos: 0x1f8, spr_ba: 0b00000000 }, // 13
    Cyc { flags: FETCH_BA|UPDATE_VC, phi1: Phi1::Refresh, xpos: 0x000, spr_ba: 0b00000000 }, // 14
    Cyc { flags: VISIBLE|FETCH_BA|SPR_CRUNCH, phi1: Phi1::Refresh, xpos: 0x008, spr_ba: 0b00000000 }, // 15
    Cyc { flags: VISIBLE|FETCH_BA|UPDATE_MCBASE, phi1: Phi1::FetchG, xpos: 0x010, spr_ba: 0b00000000 }, // 16
    Cyc { flags: VISIBLE|FETCH_BA|BRD_L1, phi1: Phi1::FetchG, xpos: 0x018, spr_ba: 0b00000000 }, // 17
    Cyc { flags: VISIBLE|FETCH_BA|BRD_L0, phi1: Phi1::FetchG, xpos: 0x020, spr_ba: 0b00000000 }, // 18
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x028, spr_ba: 0b00000000 }, // 19
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x030, spr_ba: 0b00000000 }, // 20
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x038, spr_ba: 0b00000000 }, // 21
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x040, spr_ba: 0b00000000 }, // 22
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x048, spr_ba: 0b00000000 }, // 23
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x050, spr_ba: 0b00000000 }, // 24
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x058, spr_ba: 0b00000000 }, // 25
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x060, spr_ba: 0b00000000 }, // 26
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x068, spr_ba: 0b00000000 }, // 27
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x070, spr_ba: 0b00000000 }, // 28
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x078, spr_ba: 0b00000000 }, // 29
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x080, spr_ba: 0b00000000 }, // 30
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x088, spr_ba: 0b00000000 }, // 31
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x090, spr_ba: 0b00000000 }, // 32
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x098, spr_ba: 0b00000000 }, // 33
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0a0, spr_ba: 0b00000000 }, // 34
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0a8, spr_ba: 0b00000000 }, // 35
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0b0, spr_ba: 0b00000000 }, // 36
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0b8, spr_ba: 0b00000000 }, // 37
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0c0, spr_ba: 0b00000000 }, // 38
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0c8, spr_ba: 0b00000000 }, // 39
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0d0, spr_ba: 0b00000000 }, // 40
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0d8, spr_ba: 0b00000000 }, // 41
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0e0, spr_ba: 0b00000000 }, // 42
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0e8, spr_ba: 0b00000000 }, // 43
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0f0, spr_ba: 0b00000000 }, // 44
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x0f8, spr_ba: 0b00000000 }, // 45
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x100, spr_ba: 0b00000000 }, // 46
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x108, spr_ba: 0b00000000 }, // 47
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x110, spr_ba: 0b00000000 }, // 48
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x118, spr_ba: 0b00000000 }, // 49
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x120, spr_ba: 0b00000000 }, // 50
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x128, spr_ba: 0b00000000 }, // 51
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x130, spr_ba: 0b00000000 }, // 52
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x138, spr_ba: 0b00000000 }, // 53
    Cyc { flags: VISIBLE|FETCH_BA, phi1: Phi1::FetchG, xpos: 0x140, spr_ba: 0b00000000 }, // 54
    Cyc { flags: 0, phi1: Phi1::FetchG, xpos: 0x148, spr_ba: 0b00000000 }, // 55
    Cyc { flags: SPR_DMA|SPR_EXP|BRD_R0, phi1: Phi1::Idle, xpos: 0x150, spr_ba: 0b00000001 }, // 56
    Cyc { flags: SPR_DMA|BRD_R1, phi1: Phi1::Idle, xpos: 0x158, spr_ba: 0b00000001 }, // 57
    Cyc { flags: UPDATE_RC, phi1: Phi1::Idle, xpos: 0x160, spr_ba: 0b00000011 }, // 58
    Cyc { flags: SPR_DISP, phi1: Phi1::SprPtr(0), xpos: 0x168, spr_ba: 0b00000011 }, // 59
    Cyc { flags: 0, phi1: Phi1::SprDma1(0), xpos: 0x170, spr_ba: 0b00000111 }, // 60
    Cyc { flags: 0, phi1: Phi1::SprPtr(1), xpos: 0x178, spr_ba: 0b00000110 }, // 61
    Cyc { flags: 0, phi1: Phi1::SprDma1(1), xpos: 0x180, spr_ba: 0b00001110 }, // 62
    Cyc { flags: 0, phi1: Phi1::SprPtr(2), xpos: 0x180, spr_ba: 0b00001100 }, // 63
    Cyc { flags: 0, phi1: Phi1::SprDma1(2), xpos: 0x188, spr_ba: 0b00011100 }, // 64
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The PAL table as this emulator built it before the tables were
    /// converted from VICE (from Bauer's description), verified against
    /// VICE by the pixel tests: the converted table must be the same.
    const fn build_cycles() -> [Cyc; 63] {
        let mut t = [Cyc { flags: 0, phi1: Phi1::Idle, xpos: 0, spr_ba: 0 }; 63];
        let ptr = [58u16, 60, 62, 1, 3, 5, 7, 9];
        let mut k = 0;
        while k < 63 {
            let b: u16 = if k == 0 { 63 } else { k as u16 };
            let mut f = 0u16;
            if b >= 15 && b <= 54 { f |= VISIBLE; }
            if b >= 12 && b <= 54 { f |= FETCH_BA; }
            if b == 14 { f |= UPDATE_VC; }
            if b == 15 { f |= SPR_CRUNCH; }
            if b == 16 { f |= UPDATE_MCBASE; }
            if b == 17 { f |= BRD_L1; }
            if b == 18 { f |= BRD_L0; }
            if b == 55 { f |= SPR_DMA; }
            if b == 56 { f |= SPR_DMA | SPR_EXP | BRD_R0; }
            if b == 57 { f |= BRD_R1; }
            if b == 58 { f |= SPR_DISP | UPDATE_RC; }
            let mut phi1 = Phi1::Idle;
            if b >= 16 && b <= 55 { phi1 = Phi1::FetchG; }
            if b >= 11 && b <= 15 { phi1 = Phi1::Refresh; }
            let mut spr_ba = 0u8;
            let mut n = 0;
            while n < 8 {
                if b == ptr[n] { phi1 = Phi1::SprPtr(n as u8); }
                if b == ptr[n] % 63 + 1 { phi1 = Phi1::SprDma1(n as u8); }
                // BA: 3 cycles of warning plus the two access cycles, from cycle 55+2n
                let first = 55 + 2 * n as u16;
                let mut i = 0;
                while i < 5 {
                    if b == (first - 1 + i) % 63 + 1 { spr_ba |= 1 << n; }
                    i += 1;
                }
                n += 1;
            }
            t[k] = Cyc { flags: f, phi1, xpos: (400 + 8 * (b - 1)) % 504, spr_ba };
            k += 1;
        }
        t
    }
    
    const CYCLES: [Cyc; 63] = build_cycles();

    #[test]
    fn pal_table_matches_the_formulas() {
        let built = build_cycles();
        for k in 0..63 {
            assert_eq!(PAL[k], built[k], "cycle {k}");
        }
    }

    #[test]
    fn ntsc_table() {
        // Same X and fetches as PAL from the wrap of X to 0 (cycle 14) to
        // cycle 55, then two more cycles before the sprites (before the
        // wrap X is 8 higher: the line is 520 pixels); cycles 62 and 63
        // have the same X
        for k in 14..=55 {
            assert_eq!(NTSC[k].xpos, PAL[k].xpos, "cycle {k}");
            assert_eq!(NTSC[k].phi1, PAL[k].phi1, "cycle {k}");
        }
        assert_eq!(NTSC[59].phi1, Phi1::SprPtr(0));
        assert_eq!(NTSC[62].xpos, NTSC[63].xpos);
    }
}
