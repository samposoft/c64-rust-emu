// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! The VIC-II models, as VICE's `vicii-chip-model.c` sets them up:
//!
//! | Chip | Machine | Standard | Colors resolved | Lumas |
//! |---|---|---|---|---|
//! | 6569R1 | first PAL C64s (1982-83) | PAL | a pixel late | 5 |
//! | 6569 (R3-R5) | PAL C64 | PAL | a pixel late | 9 |
//! | 8565 | PAL C64C and C64 II | PAL | at once: grey dots | 9 |
//! | 6567R56A | first NTSC C64s | old NTSC | a pixel late | 5 |
//! | 6567R8 | NTSC C64 | NTSC | a pixel late | 9 |
//! | 8562 | NTSC C64C | NTSC | at once: grey dots | 9 |
//! | 6572 | Drean C64 (Argentina) | PAL-N | a pixel late | 9 |
//!
//! The NMOS chips (656x) take a pixel to switch to a new color after a write
//! to a color register; the HMOS 856x switch at once, but show light grey
//! for the first pixel ("grey dots"), and they latch some mode bits at
//! other pixels. The first revisions have only 5 luma levels.

use crate::timing::Standard;

/// A VIC-II model.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Chip {
    Mos6569R1,
    #[default]
    Mos6569,
    Mos8565,
    Mos6567R56A,
    Mos6567R8,
    Mos8562,
    Mos6572,
}

impl Chip {
    /// Names accepted by `parse`, for help and error messages.
    pub const NAMES: &'static str = "6569, 6569r1, 8565 (PAL); 6567, 6567r56a, 8562 (NTSC); 6572 (PAL-N)";

    pub const ALL: [Chip; 7] = [
        Chip::Mos6569R1, Chip::Mos6569, Chip::Mos8565, Chip::Mos6567R56A, Chip::Mos6567R8, Chip::Mos8562, Chip::Mos6572,
    ];

    pub fn parse(s: &str) -> Option<Chip> {
        match s.to_ascii_lowercase().trim_start_matches("mos") {
            "6569r1" => Some(Chip::Mos6569R1),
            "6569" | "6569r3" | "6569r4" | "6569r5" => Some(Chip::Mos6569),
            "8565" => Some(Chip::Mos8565),
            "6567r56a" => Some(Chip::Mos6567R56A),
            "6567" | "6567r8" => Some(Chip::Mos6567R8),
            "8562" => Some(Chip::Mos8562),
            "6572" => Some(Chip::Mos6572),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Chip::Mos6569R1 => "6569R1",
            Chip::Mos6569 => "6569",
            Chip::Mos8565 => "8565",
            Chip::Mos6567R56A => "6567R56A",
            Chip::Mos6567R8 => "6567R8",
            Chip::Mos8562 => "8562",
            Chip::Mos6572 => "6572",
        }
    }

    /// The chip of a standard's usual machine: the 6569 for PAL, the 6567R8
    /// for NTSC, the only ones of old NTSC and PAL-N.
    pub fn of(standard: Standard) -> Chip {
        match standard {
            Standard::Pal => Chip::Mos6569,
            Standard::Ntsc => Chip::Mos6567R8,
            Standard::NtscOld => Chip::Mos6567R56A,
            Standard::PalN => Chip::Mos6572,
        }
    }

    pub fn standard(self) -> Standard {
        match self {
            Chip::Mos6569R1 | Chip::Mos6569 | Chip::Mos8565 => Standard::Pal,
            Chip::Mos6567R8 | Chip::Mos8562 => Standard::Ntsc,
            Chip::Mos6567R56A => Standard::NtscOld,
            Chip::Mos6572 => Standard::PalN,
        }
    }

    /// HMOS chip (8565, 8562): colors switch without the pixel of latency,
    /// with the grey dot (VICE's `color_latency` 0).
    pub fn hmos(self) -> bool {
        matches!(self, Chip::Mos8565 | Chip::Mos8562)
    }

    /// First revision, with 5 luma levels instead of 9.
    pub fn old_luma(self) -> bool {
        matches!(self, Chip::Mos6569R1 | Chip::Mos6567R56A)
    }

    /// First revisions: the light pen IRQ comes only from the trigger at
    /// the start of the frame, with the line still low (VICE's
    /// lightpen_old_irq_mode).
    pub fn old_lightpen_irq(self) -> bool {
        matches!(self, Chip::Mos6569R1 | Chip::Mos6567R56A)
    }

    /// Luma levels of the 16 colors, 0-32 (Pepto's colodore model; the first
    /// revisions: his levels for them, VICE's `vicii_colors_old`).
    pub fn luma(self) -> &'static [u8; 16] {
        if self.old_luma() { &LUMA_OLD } else { &LUMA }
    }

    /// Colors of the framebuffer (ARGB).
    pub fn palette(self) -> &'static [u32; 16] {
        if self.old_luma() { &super::C64_PALETTE_OLD } else { &super::C64_PALETTE }
    }
}

/// Luma levels (0-32) of the colodore model, 6569R3 and later.
const LUMA: [u8; 16] = [0, 32, 10, 20, 12, 16, 8, 24, 12, 8, 16, 10, 15, 24, 15, 20];
/// Luma levels of the first revisions: 0, 8, 16, 24, 32 only.
const LUMA_OLD: [u8; 16] = [0, 32, 8, 24, 16, 16, 8, 24, 16, 8, 16, 8, 16, 24, 16, 24];

crate::snapshot::impl_state_enum!(Chip { Mos6569R1, Mos6569, Mos8565, Mos6567R56A, Mos6567R8, Mos8562, Mos6572 });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for chip in Chip::ALL {
            assert_eq!(Chip::parse(chip.name()), Some(chip));
            assert_eq!(Chip::of(chip.standard()).standard(), chip.standard());
        }
        assert_eq!(Chip::parse("6567"), Some(Chip::Mos6567R8));
        assert_eq!(Chip::parse("MOS6569R5"), Some(Chip::Mos6569));
        assert_eq!(Chip::parse("6566"), None);
    }

    #[test]
    fn old_luma_has_five_levels() {
        let mut levels: Vec<u8> = LUMA_OLD.to_vec();
        levels.sort();
        levels.dedup();
        assert_eq!(levels, [0, 8, 16, 24, 32]);
        // All 16 colors stay distinct in the old palette
        let mut p = crate::vic::C64_PALETTE_OLD.to_vec();
        p.sort();
        p.dedup();
        assert_eq!(p.len(), 16);
    }
}
