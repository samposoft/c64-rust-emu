// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Video standard of the machine, set by its VIC-II (`vic::Chip`), and the
//! timings that follow from it, as in VICE's `machine_change_timing` (c64.c)
//! and `c64.h`:
//!
//! | | PAL | NTSC | Old NTSC | PAL-N |
//! |---|---|---|---|---|
//! | VIC-II | 6569, 8565 | 6567R8, 8562 | 6567R56A | 6572 (Drean) |
//! | Crystal | 17.734475 MHz | 14.31818 MHz | 14.31818 MHz | 14.328225 MHz |
//! | Clock | 985,248 Hz (/18) | 1,022,730 Hz (/14) | 1,022,730 Hz | 1,023,440 Hz (/14) |
//! | Cycles per line | 63 | 65 | 64 | 65 |
//! | Raster lines | 312 | 263 | 262 | 312 |
//! | Frames per second | 50.12 | 59.83 | 60.99 | 50.47 |
//! | Mains (CIA TOD input) | 50 Hz | 60 Hz | 60 Hz | 50 Hz |
//!
//! ROMs, SID, CIAs and the 1541 are the same: the KERNAL tells PAL from
//! NTSC at boot (flag at $02A6) from the number of raster lines, so the
//! PAL-N machine counts as PAL and the old NTSC one as NTSC.

use crate::vic::WIDTH;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Standard {
    /// PAL C64: VIC-II 6569 or 8565.
    #[default]
    Pal,
    /// NTSC C64: VIC-II 6567R8 or 8562.
    Ntsc,
    /// The first NTSC C64s (1982-83): VIC-II 6567R56A.
    NtscOld,
    /// PAL-N C64 (Drean, Argentina): VIC-II 6572.
    PalN,
}

impl Standard {
    pub fn name(self) -> &'static str {
        match self {
            Standard::Pal => "PAL",
            Standard::Ntsc => "NTSC",
            Standard::NtscOld => "old NTSC",
            Standard::PalN => "PAL-N",
        }
    }

    /// NTSC color (the NTSC and old NTSC machines).
    pub fn ntsc(self) -> bool {
        matches!(self, Standard::Ntsc | Standard::NtscOld)
    }

    /// CPU clock, Hz.
    pub fn clock_hz(self) -> u64 {
        match self {
            Standard::Pal => 985_248,
            Standard::Ntsc | Standard::NtscOld => 1_022_730,
            Standard::PalN => 1_023_440,
        }
    }

    pub fn cycles_per_line(self) -> u16 {
        match self {
            Standard::Pal => 63,
            Standard::Ntsc | Standard::PalN => 65,
            Standard::NtscOld => 64,
        }
    }

    pub fn raster_lines(self) -> u16 {
        match self {
            Standard::Pal | Standard::PalN => 312,
            Standard::Ntsc => 263,
            Standard::NtscOld => 262,
        }
    }

    /// Mains frequency, Hz: the 50/60 Hz input of the CIAs' TOD clocks.
    pub fn mains_hz(self) -> u32 {
        if self.ntsc() { 60 } else { 50 }
    }

    /// Cycles of a VIC frame.
    pub fn frame_cycles(self) -> u32 {
        self.cycles_per_line() as u32 * self.raster_lines() as u32
    }

    /// Cycles of an emulation time slice: 1/50 or 1/60 of a second, the
    /// pace of the frontends.
    pub fn slice_cycles(self) -> u32 {
        (self.clock_hz() / self.mains_hz() as u64) as u32
    }

    /// Duration of a time slice.
    pub fn slice(self) -> std::time::Duration {
        std::time::Duration::from_nanos(1_000_000_000 * self.slice_cycles() as u64 / self.clock_hz())
    }

    /// First raster line of the framebuffer. PAL and PAL-N: line 9 to 292,
    /// the lines a monitor shows. NTSC: line 22 to the last and then the
    /// first lines of the next frame (0-11, old NTSC 0-12), which the
    /// monitor shows below them (the vertical retrace comes after), as
    /// VICE's "full" border mode.
    pub fn first_fb_line(self) -> u16 {
        if self.ntsc() { 22 } else { 9 }
    }

    /// Lines of the framebuffer (WIDTH pixels each).
    pub fn fb_height(self) -> usize {
        if self.ntsc() { 253 } else { 284 }
    }

    /// Size of the framebuffer, pixels.
    pub fn fb_len(self) -> usize {
        WIDTH * self.fb_height()
    }

    /// Framebuffer row of raster line `line`, if shown.
    pub fn fb_row(self, line: u16) -> Option<usize> {
        let lines = self.raster_lines();
        let row = (line + lines - self.first_fb_line()) % lines;
        ((row as usize) < self.fb_height()).then_some(row as usize)
    }

    /// Raster line at whose end the picture in the framebuffer is complete
    /// (the VIC's frame_done): the last line of the frame (PAL), or the
    /// last one shown of the next frame (NTSC: 11, old NTSC: 12).
    pub fn frame_end_line(self) -> u16 {
        let last = self.first_fb_line() as usize + self.fb_height() - 1;
        let lines = self.raster_lines() as usize;
        if last >= lines { (last - lines) as u16 } else { lines as u16 - 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timings() {
        assert_eq!(Standard::Pal.frame_cycles(), 19656);
        assert_eq!(Standard::Ntsc.frame_cycles(), 17095);
        assert_eq!(Standard::Pal.slice_cycles(), 19704);
        assert_eq!(Standard::Ntsc.slice_cycles(), 17045);
        // 50.12 and 59.83 frames per second
        let fps = |s: Standard| s.clock_hz() as f64 / s.frame_cycles() as f64;
        assert!((fps(Standard::Pal) - 50.125).abs() < 0.001);
        assert!((fps(Standard::Ntsc) - 59.826).abs() < 0.001);
        assert_eq!(Standard::NtscOld.frame_cycles(), 16768);
        assert_eq!(Standard::PalN.frame_cycles(), 20280);
        assert!((fps(Standard::NtscOld) - 60.993).abs() < 0.001);
        assert!((fps(Standard::PalN) - 50.466).abs() < 0.001);
    }

    #[test]
    fn framebuffer_rows() {
        let pal = Standard::Pal;
        assert_eq!(pal.fb_row(9), Some(0));
        assert_eq!(pal.fb_row(292), Some(283));
        assert_eq!(pal.fb_row(293), None);
        assert_eq!(pal.fb_row(8), None);
        assert_eq!(pal.frame_end_line(), 311);
        let ntsc = Standard::Ntsc;
        assert_eq!(ntsc.fb_row(22), Some(0));
        assert_eq!(ntsc.fb_row(262), Some(240));
        // Lines 0-11 of the next frame are shown below line 262
        assert_eq!(ntsc.fb_row(0), Some(241));
        assert_eq!(ntsc.fb_row(11), Some(252));
        assert_eq!(ntsc.fb_row(12), None);
        assert_eq!(ntsc.fb_row(21), None);
        assert_eq!(ntsc.frame_end_line(), 11);
        // The 25-row display starts at raster line 51 in both
        assert_eq!(pal.fb_row(51), Some(42));
        assert_eq!(ntsc.fb_row(51), Some(29));
        // Old NTSC: 262 lines, one more of the next frame shown
        let old = Standard::NtscOld;
        assert_eq!(old.fb_row(261), Some(239));
        assert_eq!(old.fb_row(0), Some(240));
        assert_eq!(old.fb_row(12), Some(252));
        assert_eq!(old.fb_row(13), None);
        assert_eq!(old.frame_end_line(), 12);
        assert_eq!(Standard::PalN.frame_end_line(), 311);
    }
}

crate::snapshot::impl_state_enum!(Standard { Pal, Ntsc, NtscOld, PalN });
