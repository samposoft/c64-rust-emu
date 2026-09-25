// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (core/flash040core.c); see CREDITS.md.

//! AMD/Macronix parallel flash (Am29F040, Am29F040B, MX29F800CB in byte
//! mode) of the EasyFlash, GMod2 and Megabyter cartridges.
//!
//! The chip reads like a ROM; commands are write sequences with two unlock
//! cycles (AA at the "magic 1" address, 55 at "magic 2"):
//!
//! | 3rd cycle at magic 1  | command                                          |
//! |-----------------------|--------------------------------------------------|
//! | A0, then address/data | program a byte (bits can only go from 1 to 0)    |
//! | 80, AA, 55, 10        | erase the whole chip ($FF)                       |
//! | 80, AA, 55, then 30 at a sector address | erase the sector               |
//! | 90                    | autoselect: manufacturer at +0, device at +1     |
//! | F0 (also without unlock from autoselect or error) | back to read mode  |
//!
//! During an erase, reads return the status: DQ7 = 0, DQ6 changes on every
//! read (toggle bit), DQ3 = 1 after the window in which further sectors can
//! be added (30 at another address). A byte is programmed immediately; if
//! the data cannot be obtained (bits from 0 to 1) the chip stays in error
//! and returns the status until it receives F0.
//!
//! Timings are VICE's, in clock cycles: the erase is carried out once the
//! time has elapsed, on the next access.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    /// Am29F040 (GMod2): unlock at $5555/$2AAA.
    Am29F040,
    /// Am29F040B (EasyFlash): unlock at $555/$2AA.
    Am29F040B,
    /// MX29F800CB (Megabyter), byte mode: unlock at $AAA/$555, bottom boot
    /// sectors (16K, 8K, 8K, 32K, then 64K).
    Mx29F800CB,
}

struct Params {
    manufacturer: u8,
    device: u8,
    size: usize,
    magic1: u32,
    magic2: u32,
    magic_mask: u32,
    /// Window for adding sectors, duration of a sector and of a chip erase.
    erase_timeout: u64,
    erase_sector: u64,
    erase_chip: u64,
}

impl Chip {
    fn params(self) -> Params {
        match self {
            Chip::Am29F040 => Params {
                manufacturer: 0x01, device: 0xA4, size: 0x80000,
                magic1: 0x5555, magic2: 0x2AAA, magic_mask: 0x7FFF,
                erase_timeout: 80, erase_sector: 2_000_000, erase_chip: 14_000_000,
            },
            Chip::Am29F040B => Params {
                manufacturer: 0x01, device: 0xA4, size: 0x80000,
                magic1: 0x555, magic2: 0x2AA, magic_mask: 0x7FF,
                erase_timeout: 50, erase_sector: 1_000_000, erase_chip: 8_000_000,
            },
            Chip::Mx29F800CB => Params {
                manufacturer: 0xC2, device: 0x58, size: 0x100000,
                magic1: 0xAAA, magic2: 0x555, magic_mask: 0xFFF,
                erase_timeout: 40, erase_sector: 700_000, erase_chip: 8_000_000,
            },
        }
    }

    pub fn size(self) -> usize {
        self.params().size
    }

    /// Sector containing `addr`: (number, start, length).
    fn sector(self, addr: usize) -> (u32, usize, usize) {
        match self {
            Chip::Mx29F800CB if addr < 0x10000 => match addr {
                0x0000..=0x3FFF => (0, 0x0000, 0x4000),
                0x4000..=0x5FFF => (1, 0x4000, 0x2000),
                0x6000..=0x7FFF => (2, 0x6000, 0x2000),
                _ => (3, 0x8000, 0x8000),
            },
            Chip::Mx29F800CB => (3 + (addr >> 16) as u32, addr & !0xFFFF, 0x10000),
            _ => ((addr >> 16) as u32, addr & !0xFFFF, 0x10000),
        }
    }

    fn sector_start(self, n: u32) -> usize {
        match self {
            Chip::Mx29F800CB => match n {
                0 => 0x0000,
                1 => 0x4000,
                2 => 0x6000,
                3 => 0x8000,
                n => ((n - 3) as usize) << 16,
            },
            _ => (n as usize) << 16,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Read,
    Magic1,
    Magic2,
    Autoselect,
    ByteProgram,
    ByteProgramError,
    EraseMagic1,
    EraseMagic2,
    EraseSelect,
    ChipErase,
    SectorEraseTimeout,
    SectorErase,
    SectorEraseSuspend,
}

pub struct Flash {
    pub chip: Chip,
    state: State,
    /// State to return to after a command (read or autoselect).
    base: State,
    /// Last programmed byte; during an erase, the status bits.
    status: u8,
    /// Sectors to erase (one bit per sector).
    erase_mask: u32,
    /// Cycle at which the current erase phase ends.
    until: u64,
    /// The contents changed since the last save to file.
    pub dirty: bool,
}

impl Flash {
    pub fn new(chip: Chip) -> Self {
        Self {
            chip,
            state: State::Read,
            base: State::Read,
            status: 0,
            erase_mask: 0,
            until: 0,
            dirty: false,
        }
    }

    /// State after a reset. On the real chip the C64 reset does not reach
    /// the flash (an erase in progress continues): only the command
    /// interpreter goes back to read mode, as in VICE.
    pub fn reset(&mut self) {
        self.state = State::Read;
        self.base = State::Read;
        self.status = 0;
        self.erase_mask = 0;
    }

    fn magic1(&self, addr: usize) -> bool {
        let p = self.chip.params();
        addr as u32 & p.magic_mask == p.magic1
    }

    fn magic2(&self, addr: usize) -> bool {
        let p = self.chip.params();
        addr as u32 & p.magic_mask == p.magic2
    }

    /// Advances an erase up to cycle `now`.
    fn sync(&mut self, data: &mut [u8], now: u64) {
        let p = self.chip.params();
        while now >= self.until {
            match self.state {
                State::SectorEraseTimeout => {
                    self.state = State::SectorErase;
                    self.until += p.erase_sector;
                }
                State::SectorErase => {
                    let n = self.erase_mask.trailing_zeros();
                    self.erase_mask &= !(1 << n);
                    let start = self.chip.sector_start(n);
                    let (_, _, len) = self.chip.sector(start);
                    data[start..start + len].fill(0xFF);
                    self.dirty = true;
                    if self.erase_mask == 0 {
                        self.state = self.base;
                        return;
                    }
                    self.until += p.erase_sector;
                }
                State::ChipErase => {
                    data.fill(0xFF);
                    self.dirty = true;
                    self.state = self.base;
                    return;
                }
                _ => return,
            }
        }
    }

    fn erasing(&self) -> bool {
        matches!(self.state, State::ChipErase | State::SectorEraseTimeout | State::SectorErase)
    }

    /// CPU read at chip address `addr`.
    #[inline]
    pub fn read(&mut self, data: &mut [u8], addr: usize, now: u64) -> u8 {
        if self.state == State::Read {
            return data[addr];
        }
        if self.erasing() {
            self.sync(data, now);
        }
        match self.state {
            State::Autoselect => match addr & 0xFF {
                0 => self.chip.params().manufacturer,
                1 => self.chip.params().device,
                2 => 0, // sector not protected
                _ => data[addr],
            },
            State::ByteProgramError => {
                // DQ7 = inverted data, DQ6 toggles every 2 cycles, DQ5 = timeout
                (!self.status & 0x80) | ((now & 2) << 5) as u8 | 0x20
            }
            State::ChipErase | State::SectorErase | State::SectorEraseTimeout
            | State::SectorEraseSuspend => {
                let v = self.status;
                self.status ^= 0x40;
                if self.state == State::SectorEraseTimeout { v } else { v | 0x08 }
            }
            // A read during a command sequence does not interrupt it
            _ => data[addr],
        }
    }

    /// CPU write at chip address `addr`.
    pub fn write(&mut self, data: &mut [u8], addr: usize, v: u8, now: u64) {
        if self.erasing() {
            self.sync(data, now);
        }
        let p = self.chip.params();
        self.state = match self.state {
            State::Read => {
                if self.magic1(addr) && v == 0xAA { State::Magic1 } else { State::Read }
            }
            State::Magic1 => {
                if self.magic2(addr) && v == 0x55 { State::Magic2 } else { self.base }
            }
            State::Magic2 if self.magic1(addr) => match v {
                0x90 => {
                    self.base = State::Autoselect;
                    State::Autoselect
                }
                0xF0 => {
                    self.base = State::Read;
                    State::Read
                }
                0xA0 => State::ByteProgram,
                0x80 => State::EraseMagic1,
                _ => self.base,
            },
            State::Magic2 => self.base,
            State::ByteProgram => {
                // Programming only takes bits from 1 to 0
                let new = data[addr] & v;
                data[addr] = new;
                self.status = v;
                self.dirty = true;
                if new == v { self.base } else { State::ByteProgramError }
            }
            State::EraseMagic1 => {
                if self.magic1(addr) && v == 0xAA { State::EraseMagic2 } else { self.base }
            }
            State::EraseMagic2 => {
                if self.magic2(addr) && v == 0x55 { State::EraseSelect } else { self.base }
            }
            State::EraseSelect => {
                if self.magic1(addr) && v == 0x10 {
                    self.status = 0;
                    self.until = now + p.erase_chip;
                    State::ChipErase
                } else if v == 0x30 {
                    self.erase_mask = 1 << self.chip.sector(addr).0;
                    self.status = 0;
                    self.until = now + p.erase_timeout;
                    State::SectorEraseTimeout
                } else {
                    self.base
                }
            }
            State::SectorEraseTimeout => {
                if v == 0x30 {
                    self.erase_mask |= 1 << self.chip.sector(addr).0;
                    State::SectorEraseTimeout
                } else {
                    self.erase_mask = 0;
                    self.base
                }
            }
            State::SectorErase => {
                if v == 0xB0 {
                    // Suspend: the remaining time is kept
                    self.until -= now.min(self.until);
                    State::SectorEraseSuspend
                } else {
                    State::SectorErase
                }
            }
            State::SectorEraseSuspend => {
                if v == 0x30 {
                    self.until += now;
                    State::SectorErase
                } else {
                    State::SectorEraseSuspend
                }
            }
            State::Autoselect | State::ByteProgramError => {
                if v == 0xF0 {
                    self.base = State::Read;
                    State::Read
                } else if self.magic1(addr) && v == 0xAA {
                    State::Magic1
                } else {
                    self.state
                }
            }
            State::ChipErase => State::ChipErase,
        };
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

crate::snapshot::impl_state_enum!(Chip { Am29F040, Am29F040B, Mx29F800CB });
crate::snapshot::impl_state_enum!(State {
    Read, Magic1, Magic2, Autoselect, ByteProgram, ByteProgramError, EraseMagic1, EraseMagic2,
    EraseSelect, ChipErase, SectorEraseTimeout, SectorErase, SectorEraseSuspend,
});
crate::snapshot::impl_state!(Flash { chip, state, base, status, erase_mask, until, dirty });

impl Default for Flash {
    fn default() -> Self {
        Self::new(Chip::Am29F040B)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(f: &mut Flash, d: &mut [u8], c: u8, now: u64) {
        f.write(d, 0x555, 0xAA, now);
        f.write(d, 0x2AA, 0x55, now);
        f.write(d, 0x555, c, now);
    }

    #[test]
    fn program_autoselect_and_sector_erase() {
        let mut d = vec![0xFFu8; 0x80000];
        let mut f = Flash::new(Chip::Am29F040B);
        cmd(&mut f, &mut d, 0xA0, 0);
        f.write(&mut d, 0x12345, 0x5A, 0);
        assert_eq!(f.read(&mut d, 0x12345, 0), 0x5A);

        // Programming a 1 over a 0 is an error: status (DQ7 = inverted
        // data, DQ5 = 1) until F0 arrives
        cmd(&mut f, &mut d, 0xA0, 0);
        f.write(&mut d, 0x12345, 0xA5, 0);
        assert_eq!(f.read(&mut d, 0x12345, 0) & 0xA0, 0x20);
        f.write(&mut d, 0, 0xF0, 0);
        assert_eq!(f.read(&mut d, 0x12345, 0), 0x00);

        cmd(&mut f, &mut d, 0x90, 0);
        assert_eq!((f.read(&mut d, 0x20000, 0), f.read(&mut d, 0x20001, 0)), (0x01, 0xA4));
        f.write(&mut d, 0, 0xF0, 0);

        // Sector 1 ($10000-$1FFFF): status with toggle bit, then erased
        cmd(&mut f, &mut d, 0x80, 100);
        f.write(&mut d, 0x555, 0xAA, 100);
        f.write(&mut d, 0x2AA, 0x55, 100);
        f.write(&mut d, 0x10000, 0x30, 100);
        let a = f.read(&mut d, 0x10000, 200);
        let b = f.read(&mut d, 0x10000, 201);
        assert_eq!((a ^ b) & 0x40, 0x40, "toggle bit");
        assert_eq!(a & 0x88, 0x08, "DQ7 low, DQ3 high after the window");
        assert_eq!(d[0x12345], 0x00, "not erased yet");
        assert_eq!(f.read(&mut d, 0x12345, 100 + 50 + 1_000_000), 0xFF);
        assert!(f.dirty);
    }

    #[test]
    fn multi_sector_and_chip_erase() {
        let mut d = vec![0u8; 0x80000];
        let mut f = Flash::new(Chip::Am29F040B);
        // Two sectors in the same window: one after the other
        cmd(&mut f, &mut d, 0x80, 0);
        f.write(&mut d, 0x555, 0xAA, 0);
        f.write(&mut d, 0x2AA, 0x55, 0);
        f.write(&mut d, 0x30000, 0x30, 0);
        f.write(&mut d, 0x50000, 0x30, 10);
        f.read(&mut d, 0, 50 + 1_000_000);
        assert_eq!((d[0x30000], d[0x50000]), (0xFF, 0x00), "first sector");
        f.read(&mut d, 0, 50 + 2_000_000);
        assert_eq!((d[0x3FFFF], d[0x5FFFF], d[0x40000]), (0xFF, 0xFF, 0x00));
        assert_eq!(f.read(&mut d, 0x40000, 50 + 2_000_000), 0x00, "back in read mode");

        cmd(&mut f, &mut d, 0x80, 0);
        cmd(&mut f, &mut d, 0x10, 0);
        assert_eq!(f.read(&mut d, 0x40000, 7_999_999) & 0x88, 0x08);
        assert_eq!(f.read(&mut d, 0x40000, 8_000_000), 0xFF);
        assert!(d.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn mx29f800_boot_sectors() {
        let c = Chip::Mx29F800CB;
        assert_eq!(c.sector(0x5000), (1, 0x4000, 0x2000));
        assert_eq!(c.sector(0x9000), (3, 0x8000, 0x8000));
        assert_eq!(c.sector(0x23456), (5, 0x20000, 0x10000));
        assert_eq!(c.sector_start(5), 0x20000);
    }
}
