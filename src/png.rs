// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Minimal PNG writer (8-bit RGB, deflate with LZ77 and the fixed Huffman
//! codes). Lets the debugger save the framebuffer without external
//! dependencies; a C64 screen, made of long runs of the same colors, shrinks
//! from about 340 KB to a few tens.

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data {
        a = (a + d as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encodes an ARGB8888 framebuffer as an RGB PNG.
pub fn encode_argb(fb: &[u32], width: usize, height: usize) -> Vec<u8> {
    // Scanline: filter 0 + RGB
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let p = fb[y * width + x];
            raw.push(((p >> 16) & 0xFF) as u8);
            raw.push(((p >> 8) & 0xFF) as u8);
            raw.push((p & 0xFF) as u8);
        }
    }

    // zlib: header + one deflate block + adler32
    let mut z = vec![0x78, 0x01];
    z.extend_from_slice(&deflate(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bit, RGB, deflate, filter 0, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

// ── Deflate ──────────────────────────────────────────────────────────────────

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

const WINDOW: usize = 32768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// Candidates examined for each position: enough for the repetitions of a
/// screen (the same pixel, the row above).
const MAX_CHAIN: usize = 48;

/// Bits in deflate order: from the least significant bit of each byte.
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, bits: u32, count: u32) {
        self.acc |= bits << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Huffman code: sent from its most significant bit.
    fn put_code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Literal or length symbol with the fixed codes (RFC 1951, 3.2.6).
fn put_symbol(w: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => w.put_code(0x30 + sym, 8),
        144..=255 => w.put_code(0x190 + sym - 144, 9),
        256..=279 => w.put_code(sym - 256, 7),
        _ => w.put_code(0xC0 + sym - 280, 8),
    }
}

fn put_match(w: &mut BitWriter, len: usize, dist: usize) {
    let l = LEN_BASE.iter().rposition(|&b| b as usize <= len).unwrap();
    put_symbol(w, 257 + l as u32);
    w.put((len - LEN_BASE[l] as usize) as u32, LEN_EXTRA[l] as u32);
    let d = DIST_BASE.iter().rposition(|&b| b as usize <= dist).unwrap();
    w.put_code(d as u32, 5);
    w.put((dist - DIST_BASE[d] as usize) as u32, DIST_EXTRA[d] as u32);
}

fn hash3(data: &[u8], i: usize) -> usize {
    ((data[i] as usize) << 10 ^ (data[i + 1] as usize) << 5 ^ data[i + 2] as usize) & 0x7FFF
}

/// One final deflate block with the fixed codes; greedy LZ77 over hash
/// chains of three bytes.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(data.len() / 8), acc: 0, n: 0 };
    w.put(1, 1); // BFINAL
    w.put(1, 2); // BTYPE 01: fixed codes
    let mut head = vec![usize::MAX; 0x8000];
    let mut prev = vec![usize::MAX; data.len()];
    let insert = |head: &mut [usize], prev: &mut [usize], i: usize| {
        if i + MIN_MATCH <= data.len() {
            let h = hash3(data, i);
            prev[i] = head[h];
            head[h] = i;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0, 0);
        if i + MIN_MATCH <= data.len() {
            let max = (data.len() - i).min(MAX_MATCH);
            let mut cand = head[hash3(data, i)];
            let mut chain = 0;
            while cand != usize::MAX && i - cand <= WINDOW && chain < MAX_CHAIN {
                let len = data[cand..].iter().zip(&data[i..i + max]).take_while(|(a, b)| a == b).count();
                if len > best_len {
                    (best_len, best_dist) = (len, i - cand);
                    if len == max { break; }
                }
                cand = prev[cand];
                chain += 1;
            }
        }
        if best_len >= MIN_MATCH {
            put_match(&mut w, best_len, best_dist);
            for j in i..i + best_len {
                insert(&mut head, &mut prev, j);
            }
            i += best_len;
        } else {
            put_symbol(&mut w, data[i] as u32);
            insert(&mut head, &mut prev, i);
            i += 1;
        }
    }
    put_symbol(&mut w, 256);
    w.finish()
}

pub fn write_argb(path: &str, fb: &[u32], width: usize, height: usize) -> std::io::Result<()> {
    std::fs::write(path, encode_argb(fb, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inflater for the blocks with fixed codes that `deflate` writes.
    fn inflate_fixed(z: &[u8]) -> Vec<u8> {
        let mut pos = 0usize; // in bits
        let mut bit = |n: u32| -> u32 {
            let mut v = 0;
            for k in 0..n {
                v |= ((z[pos / 8] >> (pos % 8)) as u32 & 1) << k;
                pos += 1;
            }
            v
        };
        assert_eq!(bit(1), 1, "final block");
        assert_eq!(bit(2), 1, "fixed codes");
        let mut out: Vec<u8> = Vec::new();
        loop {
            // Codes from the most significant bit: 7, 8 or 9 bits
            let mut code = 0;
            for _ in 0..7 { code = code << 1 | bit(1); }
            let sym = if code <= 0x17 {
                code + 256
            } else {
                code = code << 1 | bit(1);
                if (0x30..=0xBF).contains(&code) { code - 0x30 }
                else if (0xC0..=0xC7).contains(&code) { code - 0xC0 + 280 }
                else { (code << 1 | bit(1)) - 0x190 + 144 }
            };
            match sym {
                0..=255 => out.push(sym as u8),
                256 => return out,
                _ => {
                    let l = (sym - 257) as usize;
                    let len = LEN_BASE[l] as usize + bit(LEN_EXTRA[l] as u32) as usize;
                    let mut d = 0;
                    for _ in 0..5 { d = d << 1 | bit(1); }
                    let dist = DIST_BASE[d as usize] as usize + bit(DIST_EXTRA[d as usize] as u32) as usize;
                    for _ in 0..len {
                        out.push(out[out.len() - dist]);
                    }
                }
            }
        }
    }

    #[test]
    fn deflate_round_trip() {
        let mut data: Vec<u8> = Vec::new();
        let mut seed = 12345u32;
        for i in 0..70_000u32 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            // Runs, repetitions at a distance and noise: every kind of symbol
            data.push(match i % 5000 {
                0..=1999 => (i / 700) as u8,
                2000..=3999 => data[data.len() - 1210],
                _ => (seed >> 16) as u8,
            });
        }
        for len in [0, 1, 2, 3, 300, data.len()] {
            let z = deflate(&data[..len]);
            assert_eq!(inflate_fixed(&z), &data[..len], "length {len}");
        }
        assert!(deflate(&data).len() < data.len() / 3);
    }
}
