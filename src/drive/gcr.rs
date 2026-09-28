// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (gcr.c, diskimage/fsimage-gcr.c); see CREDITS.md.

//! Disk at the magnetic flux level: GCR tracks (Group Code Recording, 4
//! bits → 5 bits), built from a D64 or D71 image or read from a G64 or
//! G71, and converted back to the image to save writes. A D71 or G71 has
//! two sides, read by the two heads of the 1571: tracks 36-70 of a D71 are
//! tracks 1-35 of the second side.
//!
//! D64 track (as the 1541 DOS writes it and as VICE builds it):
//! for each sector SYNC (5 bytes $FF), header ($08, checksum, sector,
//! track, ID2, ID1, $0F, $0F: 10 GCR bytes), gap of 9 bytes $55, SYNC,
//! data block ($07, 256 bytes, checksum, 0, 0: 325 GCR bytes), inter-sector
//! gap (8/17/12/9 bytes depending on the zone). The rest of the track is $55.
//! Lengths per zone: 7692, 7142, 6666, 6250 bytes, i.e. the bits of one
//! revolution at 300 rpm with 3.25/3.5/3.75/4 µs cells. Each track starts
//! skewed relative to the previous one, as on a real disk.
//!
//! A D64 with errors (one byte per sector after the data) produces the bad
//! sectors that copy protections check (errors 20, 21, 22, 23, 27, 29).

use crate::disk::{sector_offset, sectors_per_track};

/// Half-tracks 0..=85 of a side; the drives use 2..=84 (tracks 1-42).
pub const HALF_TRACKS: usize = 86;
/// Tracks of a side in a D71.
const D71_SIDE_TRACKS: u8 = 35;

const GCR_ENCODE: [u8; 16] = [
    0x0A, 0x0B, 0x12, 0x13, 0x0E, 0x0F, 0x16, 0x17, 0x09, 0x19, 0x1A, 0x1B, 0x0D, 0x1D, 0x1E, 0x15,
];

/// Bytes of a standard GCR track (as the DOS writes it) for the track.
fn raw_track_size(track: u8) -> usize {
    TRACK_BYTES[speed_zone(track) as usize]
}

/// Speed zone of a track (3 = tracks 1-17, the densest).
pub fn speed_zone(track: u8) -> u8 {
    (track < 31) as u8 + (track < 25) as u8 + (track < 18) as u8
}

const TRACK_BYTES: [usize; 4] = [6250, 6666, 7142, 7692];
const SECTOR_GAP: [usize; 4] = [9, 12, 17, 8];
const HEADER_GAP: usize = 9;
const SYNC: usize = 5;
/// GCR sector without gaps and syncs: header 10 + data 325.
const SECTOR_GCR: usize = 335;

#[derive(Clone, Default, PartialEq)]
pub struct Track {
    pub data: Vec<u8>,
}

impl Track {
    pub fn bits(&self) -> usize {
        self.data.len() * 8
    }
}

/// Inserted disk: the GCR tracks per half-track (empty between tracks),
/// `HALF_TRACKS` for each side.
#[derive(Default)]
pub struct Disk {
    pub tracks: Vec<Option<Track>>,
    pub write_protected: bool,
    /// The tracks have been written by the drive.
    pub dirty: bool,
    /// The disk comes from a D64 or D71 (writes are carried back to it).
    pub from_d64: bool,
    /// Tracks of the source D64 (35 or 40) or D71 (70).
    pub d64_tracks: u8,
}

/// Offset of a sector in a D64, or in a D71 (`d71`: tracks 36-70 are the
/// second side).
fn image_offset(track: u8, sector: u8, d71: bool) -> usize {
    if d71 && track > D71_SIDE_TRACKS {
        sector_offset(D71_SIDE_TRACKS + 1, 0) + sector_offset(track - D71_SIDE_TRACKS, sector)
    } else {
        sector_offset(track, sector)
    }
}

fn encode4(src: &[u8; 4], dst: &mut [u8]) {
    let mut bits: u64 = 0;
    for b in src {
        bits = bits << 10 | (GCR_ENCODE[(b >> 4) as usize] as u64) << 5 | GCR_ENCODE[(b & 15) as usize] as u64;
    }
    for (i, d) in dst[..5].iter_mut().enumerate() {
        *d = (bits >> (32 - 8 * i)) as u8;
    }
}

/// Error codes of the extended D64 (one byte per sector).
mod err {
    pub const HEADER: u8 = 2; // 20: header not found
    pub const SYNC: u8 = 3; // 21: no sync
    pub const NOBLOCK: u8 = 4; // 22: data block not found
    pub const DCHECK: u8 = 5; // 23: data checksum
    pub const HCHECK: u8 = 9; // 27: header checksum
    pub const ID: u8 = 11; // 29: ID mismatch
}

/// One GCR sector starting at `out[0]`: sync, header, gap, sync,
/// data. `error` is the extended D64 code (1 = no error).
fn encode_sector(data: &[u8], track: u8, sector: u8, id: [u8; 2], error: u8, out: &mut [u8]) {
    let sync_byte = if error == err::SYNC { 0x55 } else { 0xFF };
    let idm = if error == err::ID { 0xFF } else { 0x00 };
    out[..SYNC].fill(sync_byte);
    let mut p = SYNC;
    let mut chk = if error == err::HCHECK { 0xFF } else { 0 };
    chk ^= sector ^ track ^ id[1] ^ id[0] ^ idm;
    let marker = if error == err::HEADER { 0xFF } else { 0x08 };
    encode4(&[marker, chk, sector, track], &mut out[p..]);
    p += 5;
    encode4(&[id[1], id[0] ^ idm, 0x0F, 0x0F], &mut out[p..]);
    p += 5 + HEADER_GAP;
    out[p..p + SYNC].fill(sync_byte);
    p += SYNC;
    let mut chk: u8 = if error == err::DCHECK { 0xFF } else { 0 };
    for b in data {
        chk ^= b;
    }
    // Data block: $07, 256 bytes, checksum, 0, 0 (with error 22 the marker
    // is $00, which in GCR starts with a 0 and does not extend the sync)
    let mut block = Vec::with_capacity(260);
    block.push(if error == err::NOBLOCK { 0x00 } else { 0x07 });
    block.extend_from_slice(data);
    block.extend_from_slice(&[chk, 0, 0]);
    for chunk in block.chunks(4) {
        encode4(chunk.try_into().unwrap(), &mut out[p..]);
        p += 5;
    }
}

impl Disk {
    /// Disk from a D64 image (35 or 40 tracks, with or without errors).
    pub fn from_d64(d64: &[u8]) -> Result<Disk, String> {
        let (tracks, has_errors) = match d64.len() {
            174_848 => (35, false),
            175_531 => (35, true),
            196_608 => (40, false),
            197_376 => (40, true),
            n => return Err(format!("D64 of {n} bytes: unknown size")),
        };
        Ok(Self::from_sectors(d64, tracks, has_errors, 1))
    }

    /// Disk from a D71 image (70 tracks on two sides, with or without
    /// errors).
    pub fn from_d71(d71: &[u8]) -> Result<Disk, String> {
        let has_errors = match d71.len() {
            349_696 => false,
            351_062 => true,
            n => return Err(format!("D71 of {n} bytes: unknown size")),
        };
        Ok(Self::from_sectors(d71, 2 * D71_SIDE_TRACKS, has_errors, 2))
    }

    /// Disk from a D64 or D71 (`sides` 2), as VICE builds it.
    fn from_sectors(image: &[u8], tracks: u8, has_errors: bool, sides: usize) -> Disk {
        let sectors_total = image_offset(tracks + 1, 0, sides == 2) / 256;
        let errors = if has_errors { &image[sectors_total * 256..] } else { &[][..] };
        let bam = sector_offset(18, 0);
        let mut id = [image[bam + 0xA2], image[bam + 0xA3]];
        // A D71 whose sides were formatted one at a time (no double-sided
        // flag in the BAM): the second side has its own ID, in its BAM on
        // track 53, and its headers count the tracks from 1 again
        let single_sides = sides == 2 && image[bam + 3] & 0x80 == 0;

        let mut disk = Disk {
            tracks: vec![None; HALF_TRACKS * sides],
            from_d64: true,
            d64_tracks: tracks,
            ..Default::default()
        };
        let mut skew = 0usize;
        for track in 1..=tracks {
            let side = (sides == 2 && track > D71_SIDE_TRACKS) as usize;
            let physical = track - side as u8 * D71_SIDE_TRACKS;
            let mut header_track = track;
            if single_sides && side == 1 {
                header_track = physical;
                if physical == 1 {
                    let bam2 = image_offset(18 + D71_SIDE_TRACKS, 0, true);
                    id = [image[bam2 + 0xA2], image[bam2 + 0xA3]];
                }
            }
            let zone = speed_zone(physical) as usize;
            let size = TRACK_BYTES[zone];
            let mut raw = vec![0x55u8; size];
            let mut p = 0;
            let n = sectors_per_track(physical);
            for sector in 0..n {
                let off = image_offset(track, sector, sides == 2);
                let index = off / 256;
                let error = errors.get(index).copied().unwrap_or(1);
                encode_sector(&image[off..off + 256], header_track, sector, id, error, &mut raw[p..]);
                p += SECTOR_GCR + HEADER_GAP + SECTOR_GAP[zone] + 2 * SYNC;
            }
            // Skew between tracks: the head step time (VICE)
            skew += p - SECTOR_GAP[zone];
            skew += size * 100 / 270;
            skew %= size;
            raw.rotate_right(skew);
            disk.tracks[side * HALF_TRACKS + physical as usize * 2] = Some(Track { data: raw });
        }
        // As in VICE: odd half-tracks have no flux (zeros, i.e.
        // random bits), tracks beyond the image up to 42 are unformatted
        // ($55: regular flux, no sync)
        for side in 0..sides {
            for track in 1..=42u8 {
                let size = raw_track_size(track);
                let even = side * HALF_TRACKS + track as usize * 2;
                if disk.tracks[even].is_none() {
                    disk.tracks[even] = Some(Track { data: vec![0x55; size] });
                }
                let len = disk.tracks[even].as_ref().unwrap().data.len();
                disk.tracks[even + 1] = Some(Track { data: vec![0; len] });
            }
        }
        disk
    }

    /// Disk from an image file: G64 or G71 (from the signature), D71 or D64
    /// (from the size).
    pub fn from_image(data: &[u8]) -> Result<Disk, String> {
        if data.starts_with(b"GCR-15") {
            Self::from_g64(data)
        } else if matches!(data.len(), 349_696 | 351_062) {
            Self::from_d71(data)
        } else {
            Self::from_d64(data)
        }
    }

    /// Sides of the disk (2 for a D71 or G71).
    pub fn sides(&self) -> usize {
        self.tracks.len() / HALF_TRACKS
    }

    /// Disk from a G64 or G71 image (GCR tracks recorded as they are; a G71
    /// has the 84 half-tracks of the second side after those of the first).
    pub fn from_g64(g64: &[u8]) -> Result<Disk, String> {
        let sides = match g64.get(..8) {
            Some(b"GCR-1541") => 1,
            Some(b"GCR-1571") => 2,
            _ => return Err("not a G64 or G71 file".into()),
        };
        if g64.len() < 12 {
            return Err("truncated G64".into());
        }
        let n = g64[9] as usize;
        let le32 = |i: usize| -> Result<usize, String> {
            g64.get(i..i + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
                .ok_or_else(|| "truncated G64".to_string())
        };
        let mut disk = Disk { tracks: vec![None; HALF_TRACKS * sides], ..Default::default() };
        for i in 0..n.min((HALF_TRACKS - 2) * sides) {
            let (side, ht) = (i / (HALF_TRACKS - 2), i % (HALF_TRACKS - 2) + 2);
            let index = side * HALF_TRACKS + ht;
            let off = le32(12 + 4 * i)?;
            if off == 0 {
                // Missing track: unformatted, as in VICE
                let size = raw_track_size((ht / 2) as u8);
                disk.tracks[index] = Some(Track { data: vec![0x55; size] });
                continue;
            }
            let len = g64.get(off..off + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
                .ok_or("truncated G64")?;
            let data = g64.get(off + 2..off + 2 + len).ok_or("truncated G64")?;
            if !data.is_empty() {
                disk.tracks[index] = Some(Track { data: data.to_vec() });
            }
        }
        Ok(disk)
    }

    /// G64 image of the tracks (84 half-tracks, speed zones according to the
    /// track); a G71 for a disk with two sides.
    pub fn to_g64(&self) -> Vec<u8> {
        let per_side = HALF_TRACKS - 2;
        let n = per_side * self.sides();
        let max = self.tracks.iter().flatten().map(|t| t.data.len()).max().unwrap_or(7928).max(7928);
        let mut out = if self.sides() == 2 { b"GCR-1571".to_vec() } else { b"GCR-1541".to_vec() };
        out.push(0);
        out.push(n as u8);
        out.extend((max as u16).to_le_bytes());
        let table = out.len();
        out.resize(table + 8 * n, 0);
        for i in 0..n {
            let ht = i % per_side + 2;
            let Some(t) = &self.tracks[i / per_side * HALF_TRACKS + ht] else { continue };
            let off = out.len() as u32;
            out[table + 4 * i..table + 4 * i + 4].copy_from_slice(&off.to_le_bytes());
            let zone = speed_zone((ht / 2) as u8) as u32;
            out[table + 4 * n + 4 * i..table + 4 * n + 4 * i + 4].copy_from_slice(&zone.to_le_bytes());
            out.extend((t.data.len() as u16).to_le_bytes());
            out.extend(&t.data);
            out.resize(out.len() + max - t.data.len(), 0);
        }
        out
    }

    /// Converts the tracks back into the D64 or D71 (to save writes).
    /// Unreadable sectors keep the contents of `original`.
    pub fn to_d64(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let tracks = self.d64_tracks.max(35);
        let sides = self.sides();
        for track in 1..=tracks {
            let side = (sides == 2 && track > D71_SIDE_TRACKS) as usize;
            let physical = track - side as u8 * D71_SIDE_TRACKS;
            let Some(t) = &self.tracks[side * HALF_TRACKS + physical as usize * 2] else { continue };
            for sector in 0..sectors_per_track(physical) {
                // Headers of a second side formatted on its own count from 1
                let data = find_sector(t, track, sector)
                    .or_else(|| (side == 1).then(|| find_sector(t, physical, sector)).flatten());
                if let Some(data) = data {
                    let off = image_offset(track, sector, sides == 2);
                    if off + 256 <= out.len() {
                        out[off..off + 256].copy_from_slice(&data);
                    }
                }
            }
        }
        out
    }
}

/// Track bit as a circular stream: bit position `i`.
fn bit(t: &Track, i: usize) -> u8 {
    let i = i % t.bits();
    (t.data[i >> 3] >> (7 - (i & 7))) & 1
}

const GCR_DECODE: [i8; 32] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 8, 0, 1, -1, 12, 4, 5,
    -1, -1, 2, 3, -1, 15, 6, 7, -1, 9, 10, 11, -1, 13, 14, -1,
];

/// Reads `n` decoded bytes starting at bit `pos`.
fn read_block(t: &Track, pos: usize, n: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(n);
    let mut p = pos;
    for _ in 0..n {
        let mut v = 0u8;
        for _ in 0..2 {
            let mut q = 0usize;
            for _ in 0..5 {
                q = q << 1 | bit(t, p) as usize;
                p += 1;
            }
            let d = GCR_DECODE[q];
            if d < 0 {
                return None;
            }
            v = v << 4 | d as u8;
        }
        out.push(v);
    }
    Some(out)
}

/// Looks for the sector header and the data block that follows it.
fn find_sector(t: &Track, track: u8, sector: u8) -> Option<Vec<u8>> {
    let bits = t.bits();
    let mut ones = 0;
    let mut i = 0;
    // Two revolutions: a sync may straddle the end of the track
    while i < 2 * bits {
        if bit(t, i) == 1 {
            ones += 1;
            i += 1;
            continue;
        }
        if ones >= 10 {
            if let Some(h) = read_block(t, i, 8) {
                if h[0] == 0x08 && h[2] == sector && h[3] == track {
                    // Next sync: the data block
                    let mut j = i + 80;
                    let mut ones2 = 0;
                    while j < i + 80 + 8 * 40 {
                        if bit(t, j) == 1 {
                            ones2 += 1;
                        } else {
                            if ones2 >= 10 {
                                let d = read_block(t, j, 257)?;
                                return (d[0] == 0x07).then(|| d[1..].to_vec());
                            }
                            ones2 = 0;
                        }
                        j += 1;
                    }
                    return None;
                }
            }
        }
        ones = 0;
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d64_round_trip_through_gcr() {
        let mut d64 = vec![0u8; 174_848];
        for (i, b) in d64.iter_mut().enumerate() {
            *b = (i * 7 + i / 256) as u8;
        }
        let disk = Disk::from_d64(&d64).unwrap();
        assert_eq!(disk.tracks[2].as_ref().unwrap().data.len(), 7692);
        assert_eq!(disk.tracks[70].as_ref().unwrap().data.len(), 6250);
        // Odd half-tracks without flux, as in VICE
        assert!(disk.tracks[3].as_ref().unwrap().data.iter().all(|&b| b == 0));
        let back = disk.to_d64(&vec![0u8; 174_848]);
        assert!(back == d64);
    }

    #[test]
    fn d71_sides_and_round_trip() {
        let mut d71 = vec![0u8; 349_696];
        for (i, b) in d71.iter_mut().enumerate() {
            *b = (i * 5 + i / 256) as u8;
        }
        // Double-sided flag in the BAM: headers of side 2 say tracks 36-70
        d71[image_offset(18, 0, true) + 3] = 0x80;
        let disk = Disk::from_d71(&d71).unwrap();
        assert_eq!(disk.sides(), 2);
        let side2_track1 = disk.tracks[HALF_TRACKS + 2].as_ref().unwrap();
        assert_eq!(side2_track1.data.len(), 7692);
        assert_eq!(find_sector(side2_track1, 36, 20).unwrap(), d71[image_offset(36, 20, true)..][..256]);
        assert!(disk.to_d64(&vec![0u8; 349_696]) == d71);
        // Sides formatted one at a time: side 2 counts from track 1 again
        d71[image_offset(18, 0, true) + 3] = 0;
        let disk = Disk::from_d71(&d71).unwrap();
        assert!(find_sector(disk.tracks[HALF_TRACKS + 2].as_ref().unwrap(), 1, 0).is_some());
        assert!(disk.to_d64(&vec![0u8; 349_696]) == d71);
        // Through a G71 and back
        let g71 = Disk::from_g64(&disk.to_g64()).unwrap();
        assert_eq!(g71.sides(), 2);
        assert!(g71.tracks == disk.tracks);
    }
}
