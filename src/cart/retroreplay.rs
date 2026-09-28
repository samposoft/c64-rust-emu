// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (c64/cart/retroreplay.c, Copyright (C)
// Andreas Boose, Groepaz, Marco van den Heuvel); see CREDITS.md.

//! Retro Replay (and Nordic Replay, CRT revision 1), by Individual
//! Computers: an Action Replay compatible freezer with 64K of ROM in eight
//! 8K banks (half of a 128K flash; the other half is reached only with
//! the bank jumper), 32K of RAM in four banks and a clock port for
//! accessories such as the RR-Net.
//!
//! - $DE00 (write): bits 0-1 /GAME and /EXROM as on the Action Replay,
//!   bit 2 disables the registers until reset, bits 3-4 and 7 the bank,
//!   bit 5 RAM instead of ROM, bit 6 acknowledges a freeze (until then the
//!   cartridge stays in Ultimax whatever bits 0-1 say).
//! - $DE01 (write): bit 0 enables the clock port at $DE02-$DE0F, bits 3-4
//!   and 7 the bank; bits 1 (AllowBank: the RAM bank also at $DE00-$DFFF),
//!   2 (NoFreeze) and 6 (REU compatible map: the cartridge moves from
//!   $DF00 to $DE00) can be written only once after reset.
//! - $DE00/$DE01 (read): the bank bits, AllowBank, the freeze button, the
//!   REU map.
//! - $DF00-$DFFF (or $DE02-$DEFF with the REU map): the last page of the
//!   bank, ROM or RAM.
//!
//! In 16K mode without RAM nothing answers at $8000 and in I/O, and while
//! frozen the cartridge shows only its ROM at $E000. $DE00 = $22, the
//! Action Replay's broken mode, turns the cartridge off with its RAM at
//! $DF00 on the Retro Replay; on the Nordic Replay $22 selects the Nordic
//! Power map, with RAM at $A000-$BFFF and ROM at $8000. The flash is never
//! written: that needs the flash jumper, which is not emulated.

use super::{CartState, Kind, Mode, BANK};

/// The freeze button reads as pressed in $DE00 bit 2 for 30 frames after
/// a press (VICE keeps it there because it does not see the release).
const BUTTON_CYCLES: u64 = 312 * 65 * 30;

/// Retro Replay registers besides mode, bank and RAM (kept in `CartState`).
#[derive(Clone, Default)]
pub struct RrState {
    /// Nordic Replay (CRT revision 1).
    pub nordic: bool,
    /// Frozen: Ultimax until $DE00 bit 6 is written.
    pub frozen: bool,
    /// $DE01 has been written since reset (bits 1, 2 and 6 are latched).
    write_once: bool,
    /// AllowBank: the RAM bank applies to $DE00-$DFFF too.
    allow_bank: bool,
    /// NoFreeze: the freeze button is ignored.
    no_freeze: bool,
    /// REU compatible map: the I/O window moves from $DF00 to $DE00.
    reu_mapping: bool,
    /// Clock port enabled at $DE02-$DE0F.
    pub clockport: bool,
    /// $DE00 = $22: Nordic Power map on the Nordic Replay, RAM at $DF00
    /// with the cartridge off on the Retro Replay.
    ram_at_a000: bool,
    /// Cycle until which the freeze button reads as pressed.
    button_until: u64,
}

impl CartState {
    fn rr_ram_index(&self, off: u16) -> usize {
        (off as usize & (BANK - 1)) + (self.roml_bank as usize & 3) * BANK
    }

    /// RAM cell for a $DE00-$DFFF access: in the RAM bank only with
    /// AllowBank.
    fn rr_io_index(&self, off: u16) -> usize {
        if self.rr.allow_bank { self.rr_ram_index(off) } else { off as usize & (BANK - 1) }
    }

    /// Nothing answers at $8000-$9FFF: frozen, or 16K mode without RAM.
    pub(super) fn rr_roml_open(&self) -> bool {
        self.rr.frozen || (!self.export_ram && !(self.rr.nordic && self.rr.ram_at_a000) && self.mode == Mode::Game16K)
    }

    pub(super) fn rr_peek_roml(&self, off: u16) -> u8 {
        if self.export_ram {
            self.ram[self.rr_ram_index(off)]
        } else {
            Self::byte(&self.roml, self.roml_bank, off)
        }
    }

    /// ROMH ($A000 in 16K, $E000 in Ultimax): the same ROM bank, but with
    /// the RAM selected and no AllowBank bits 0-1 of the bank do not
    /// count; the Nordic Power map puts RAM here.
    pub(super) fn rr_peek_romh(&self, off: u16) -> u8 {
        let rr = &self.rr;
        let rom_bank = if rr.allow_bank { self.roml_bank } else { self.roml_bank & !3 };
        if rr.nordic && rr.ram_at_a000 {
            return if rr.frozen {
                Self::byte(&self.roml, rom_bank, off)
            } else {
                self.ram[self.rr_io_index(off)]
            };
        }
        let bank = if rr.allow_bank || !self.export_ram { self.roml_bank } else { rom_bank };
        Self::byte(&self.roml, bank, off)
    }

    /// Write to $8000-$9FFF outside Ultimax: only the Nordic Replay puts it
    /// in its RAM too (the C64 RAM always gets it).
    pub(super) fn rr_write_roml_game(&mut self, off: u16, v: u8) {
        if self.rr.nordic && self.export_ram {
            let i = self.rr_ram_index(off);
            self.ram[i] = v;
        }
    }

    /// Write to $A000-$BFFF in 16K mode: in the Nordic Power map it goes to
    /// the cartridge RAM only (true).
    pub(super) fn rr_write_romh_game(&mut self, off: u16, v: u8) -> bool {
        if self.rr.nordic && self.rr.ram_at_a000 && !self.rr.frozen {
            let i = self.rr_io_index(off);
            self.ram[i] = v;
            return true;
        }
        false
    }

    /// $A000-$BFFF in Ultimax: the Nordic Replay frozen in the Nordic Power
    /// map shows its RAM, otherwise nothing answers.
    pub(super) fn rr_ultimax_a000(&self, off: u16) -> Option<u8> {
        let rr = &self.rr;
        (rr.nordic && rr.ram_at_a000 && rr.frozen).then(|| self.ram[self.rr_io_index(off)])
    }

    pub(super) fn rr_write_ultimax_a000(&mut self, off: u16, v: u8) {
        let rr = &self.rr;
        if rr.nordic && rr.ram_at_a000 && rr.frozen {
            let i = self.rr_io_index(off);
            self.ram[i] = v;
        }
    }

    /// $DE00/$DE01 read: bank bits, AllowBank, freeze button, REU map.
    fn rr_status(&self, now: u64) -> u8 {
        let b = self.roml_bank;
        ((b & 3) << 3) | ((b & 4) << 5) | ((b & 8) << 2)
            | if self.rr.allow_bank { 0x02 } else { 0 }
            | if self.rr.reu_mapping { 0x40 } else { 0 }
            | if now < self.rr.button_until { 0x04 } else { 0 }
    }

    /// The I/O window ($DE00-$DEFF with the REU map, $DF00-$DFFF
    /// otherwise): RAM, the ROM bank, or nothing (in Ultimax and in 16K
    /// without RAM, and while frozen).
    fn rr_window(&self, off: u16) -> Option<u8> {
        if self.rr.frozen {
            return None;
        }
        if self.export_ram || self.rr.ram_at_a000 {
            Some(self.ram[self.rr_io_index(off)])
        } else if !matches!(self.mode, Mode::Ultimax | Mode::Game16K) {
            Some(Self::byte(&self.roml, self.roml_bank, off))
        } else {
            None
        }
    }

    fn rr_write_window(&mut self, off: u16, v: u8) {
        if !self.rr.frozen && (self.export_ram || (self.rr.nordic && self.rr.ram_at_a000)) {
            let i = self.rr_io_index(off);
            self.ram[i] = v;
        }
    }

    pub(super) fn rr_peek_io1(&self, off: u8, now: u64) -> Option<u8> {
        match off {
            // The clock port, enabled even with the registers disabled,
            // reads 0 without an accessory (with the RR-Net the bus sends
            // these addresses to it)
            2..=0x0F if self.rr.clockport => Some(0),
            _ if self.locked => None,
            0 | 1 => Some(self.rr_status(now)),
            _ if self.rr.reu_mapping => self.rr_window(0x1E00 | off as u16),
            _ => None,
        }
    }

    pub(super) fn rr_write_io1(&mut self, off: u8, v: u8) {
        if self.locked {
            return;
        }
        match off {
            0 => {
                self.regs[0] = v;
                let bank = ((v >> 3) & 3) | ((v >> 5) & 4);
                let mut mode = Mode::from_lines(v & 0x02 == 0, v & 0x01 != 0);
                self.rr.ram_at_a000 = false;
                self.export_ram = false;
                if self.rr.nordic && v & 0x67 == 0x22 {
                    // Nordic Power: 16K with RAM at $A000
                    mode = Mode::Game16K;
                    self.rr.ram_at_a000 = true;
                } else {
                    if v & 0x40 != 0 {
                        self.rr.frozen = false;
                        self.nmi = false;
                    }
                    self.export_ram = v & 0x20 != 0;
                    if !self.rr.nordic && v & 0x67 == 0x22 {
                        mode = Mode::Off;
                        self.rr.ram_at_a000 = true;
                    }
                }
                // Until the freeze is acknowledged bits 0-1 have no effect
                if self.rr.frozen {
                    mode = Mode::Ultimax;
                }
                self.mode = mode;
                self.roml_bank = bank;
                self.romh_bank = bank;
                // The VIC (phi1) sees the cartridge as in 8K mode
                self.vic_ram = true;
                self.locked = v & 0x04 != 0;
            }
            1 => {
                self.regs[1] = v;
                if !self.rr.write_once {
                    self.rr.allow_bank = v & 0x02 != 0;
                    self.rr.no_freeze = v & 0x04 != 0;
                    self.rr.reu_mapping = v & 0x40 != 0;
                    self.rr.write_once = true;
                }
                let bank = ((v >> 3) & 3) | ((v >> 5) & 4);
                self.roml_bank = bank;
                self.romh_bank = bank;
                self.rr.clockport = v & 0x01 != 0;
            }
            2..=0x0F if self.rr.clockport => {}
            _ if self.rr.reu_mapping => self.rr_write_window(0x1E00 | off as u16, v),
            _ => {}
        }
    }

    pub(super) fn rr_peek_io2(&self, off: u8) -> Option<u8> {
        if self.locked || self.rr.reu_mapping {
            return None;
        }
        self.rr_window(0x1F00 | off as u16)
    }

    pub(super) fn rr_write_io2(&mut self, off: u8, v: u8) {
        if !self.locked && !self.rr.reu_mapping {
            self.rr_write_window(0x1F00 | off as u16, v);
        }
    }

    /// The freeze button, when it reaches the cartridge: it reads as
    /// pressed from now on, and freezes unless NoFreeze is set.
    pub(super) fn rr_freeze_allowed(&mut self, now: u64) -> bool {
        self.rr.button_until = now + BUTTON_CYCLES;
        !self.rr.no_freeze
    }

    /// Freeze (VICE's retroreplay_freeze): registers on again, Ultimax
    /// with bank 0 of the ROM at $E000, for the VIC too.
    pub(super) fn rr_freeze(&mut self) {
        self.locked = false;
        self.rr.frozen = true;
        self.mode = Mode::Ultimax;
        self.export_ram = false;
        self.roml_bank = 0;
        self.romh_bank = 0;
        self.vic_ram = false;
    }

    /// Reset: the latched bits of $DE01 and the clock port too.
    pub(super) fn rr_reset(&mut self) {
        self.rr = RrState { nordic: self.rr.nordic, ..RrState::default() };
    }

    /// Retro Replay whose clock port is enabled: $DE02-$DE0F belong to the
    /// accessory plugged into it.
    pub fn clockport(&self) -> bool {
        self.kind == Kind::RetroReplay && self.rr.clockport
    }

    pub(super) fn rr_state(&self) -> String {
        let rr = &self.rr;
        let mut s = format!("{}, RAM at $8000 {}, bank {}{}{}{}, clock port {}",
            if self.locked { "registers disabled until reset" } else { "registers enabled" },
            if self.export_ram { "on" } else { "off" },
            self.roml_bank,
            if rr.allow_bank { ", AllowBank" } else { "" },
            if rr.no_freeze { ", NoFreeze" } else { "" },
            if rr.reu_mapping { ", REU map" } else { "" },
            if rr.clockport { "on" } else { "off" });
        if rr.frozen {
            s += ", frozen";
        }
        s + if self.nmi { ", NMI held" } else { ", NMI released" }
    }
}

crate::snapshot::impl_state!(RrState {
    nordic, frozen, write_once, allow_bank, no_freeze, reu_mapping, clockport, ram_at_a000, button_until,
});
