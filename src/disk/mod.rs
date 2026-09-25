// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

/// D64 parser with KERNAL LOAD interception (HLE).
/// Format reference: https://ist.uwaterloo.ca/~schepers/formats/D64.TXT

pub fn sectors_per_track(track: u8) -> u8 {
    match track {
        1..=17  => 21,
        18..=24 => 19,
        25..=30 => 18,
        31..=40 => 17,
        _       => 0,
    }
}

pub fn sector_offset(track: u8, sector: u8) -> usize {
    let mut off = 0usize;
    for t in 1..track {
        off += sectors_per_track(t) as usize * 256;
    }
    off + sector as usize * 256
}

pub struct DirEntry {
    pub name: String,       // ASCII for display
    pub name_raw: Vec<u8>,  // raw PETSCII for comparison
    pub file_type: u8,      // 1=SEQ 2=PRG 3=USR 4=REL (bit 0-3)
    pub start_track: u8,
    pub start_sector: u8,
    pub blocks: u16,
}

#[derive(Default)]
pub struct D64 {
    data: Vec<u8>,
}

impl D64 {
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        if data.len() < 174_848 { return None; }
        Some(D64 { data })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    fn read_sector(&self, track: u8, sector: u8) -> Option<&[u8]> {
        if track == 0 || track > 40 { return None; }
        let off = sector_offset(track, sector);
        if off + 256 > self.data.len() { return None; }
        Some(&self.data[off..off + 256])
    }

    pub fn list_files(&self) -> Vec<DirEntry> {
        let mut entries = Vec::new();
        let mut track = 18u8;
        let mut sector = 1u8;

        loop {
            let blk = match self.read_sector(track, sector) {
                Some(b) => b,
                None => break,
            };

            for i in 0..8usize {
                // Directory sector layout (D64.TXT):
                //   Byte 0-1      : link to the next sector
                //   Slot i=0      : bytes 0-31 of the sector, but the useful data
                //                   starts at byte 2 (after the link)
                //   Slot i=1..7   : bytes i*32 .. i*32+31
                //
                // Inside each 32-byte slot:
                //   +0  file_type (for slot 0: absolute +2 in the sector)
                //   +1  start_track
                //   +2  start_sector
                //   +3..+18 filename (16 PETSCII bytes, $A0 padding)
                //   +30 file_size_lo
                //   +31 file_size_hi

                let slot = i * 32; // slot start in the sector (0, 32, 64, ...)

                // The first 2 bytes of each slot are the link to the next sector
                // (valid only for slot 0; 00/00 in the others). file_type is always at +2.
                let base = slot + 2;

                if base + 32 > 256 { break; }

                let ftype = blk[base];
                if ftype == 0 { continue; } // empty/deleted slot

                let start_track  = blk[base + 1];
                let start_sector = blk[base + 2];

                // Filename: 16 PETSCII bytes, terminated by the first $A0
                let raw: Vec<u8> = blk[base + 3..base + 19]
                    .iter()
                    .take_while(|&&b| b != 0xA0)
                    .cloned()
                    .collect();
                let name = raw.iter().map(|&b| petscii_to_ascii(b)).collect();

                // File size: always at offset +30 from slot_start
                // (slot 0: sector bytes 30-31; slot 1: bytes 62-63; etc.)
                let size_lo = blk[slot + 30];
                let size_hi = blk[slot + 31];

                entries.push(DirEntry {
                    name,
                    name_raw: raw,
                    file_type: ftype & 0x07,
                    start_track,
                    start_sector,
                    blocks: u16::from_le_bytes([size_lo, size_hi]),
                });
            }

            track  = blk[0];
            sector = blk[1];
            if track == 0 { break; }
        }
        entries
    }

    /// Finds a file by raw PETSCII name (as it comes from C64 RAM).
    /// Supports the `*` wildcard (first PRG) and a trailing `*` (prefix match).
    pub fn find_file(&self, name_raw: &[u8]) -> Option<DirEntry> {
        // Bare wildcard: first available PRG file
        if name_raw == b"*" || name_raw.is_empty() {
            return self.list_files()
                .into_iter()
                .find(|e| e.file_type == 2 || e.file_type == 1);
        }

        let needle = petscii_normalize(name_raw);
        let files  = self.list_files();

        // Trailing wildcard (e.g. b"LEVEL*")
        if needle.last() == Some(&b'*') {
            let prefix = &needle[..needle.len() - 1];
            return files.into_iter().find(|e| {
                petscii_normalize(&e.name_raw).starts_with(prefix)
            });
        }

        // Exact match (case-insensitive PETSCII)
        files.into_iter().find(|e| petscii_normalize(&e.name_raw) == needle)
    }

    /// Reads all the file data following the track/sector chain.
    /// The result includes the 2-byte load address (as in a standard PRG).
    pub fn read_file_data(&self, entry: &DirEntry) -> Vec<u8> {
        let mut out = Vec::new();
        let mut track  = entry.start_track;
        let mut sector = entry.start_sector;

        for _ in 0..=683 { // sector limit, for safety
            let blk = match self.read_sector(track, sector) {
                Some(b) => b,
                None => break,
            };

            let next_track  = blk[0];
            let next_sector = blk[1];

            if next_track == 0 {
                // Last block: next_sector is the (1-based) index of the last valid byte
                // VICE uses: amount = next_sector - 1 → data in blk[2..2+amount]
                let amount = (next_sector as usize).saturating_sub(1);
                out.extend_from_slice(&blk[2..2 + amount.min(254)]);
                break;
            } else {
                out.extend_from_slice(&blk[2..]); // 254 bytes of data
                track  = next_track;
                sector = next_sector;
            }
        }
        out
    }

    pub fn print_directory(&self) {
        // Hex dump of the first directory sector, for diagnostics
        if let Some(blk) = self.read_sector(18, 1) {
            eprintln!("--- hex dump track 18/1 (first 64 bytes) ---");
            for row in 0..4 {
                eprint!("  {:02X}: ", row * 16);
                for col in 0..16 {
                    eprint!("{:02X} ", blk[row * 16 + col]);
                }
                eprintln!();
            }
        }
        eprintln!("  # TYPE BLOCKS  NAME");
        for (i, e) in self.list_files().iter().enumerate() {
            let tname = match e.file_type {
                1 => "SEQ", 2 => "PRG", 3 => "USR", 4 => "REL", _ => "???",
            };
            eprintln!("{:3}  {}  {:5}  {}", i + 1, tname, e.blocks, e.name);
        }
    }
}

// ── PETSCII utilities ─────────────────────────────────────────────────────────

/// Normalizes PETSCII for comparison: converts lowercase letters→uppercase.
fn petscii_normalize(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|&b| {
        // PETSCII lowercase letters (0x61-0x7A) → uppercase (0x41-0x5A)
        if b >= 0x61 && b <= 0x7A { b - 0x20 } else { b }
    }).collect()
}

/// Converts a PETSCII byte to a printable ASCII char (for display).
fn petscii_to_ascii(b: u8) -> char {
    match b {
        0x20..=0x3F => b as char,           // punctuation and digits: same as ASCII
        0x40        => '@' as char,
        0x41..=0x5A => b as char,           // A-Z PETSCII = A-Z ASCII
        0x5B..=0x5F => b as char,
        0x61..=0x7A => (b - 0x20) as char, // PETSCII lowercase → ASCII uppercase
        _           => '?',
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

// The mounted image is part of the state: multi-load games keep loading
// from the disk after a restore.
crate::snapshot::impl_state!(D64 { data });
