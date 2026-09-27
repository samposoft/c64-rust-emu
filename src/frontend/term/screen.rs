// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Drawing the framebuffer in the terminal.
//!
//! - [`KittyScreen`]: Kitty graphics protocol. The frame goes into a POSIX
//!   shared memory object and only its name is sent to the terminal (which
//!   reads and deletes it); without shared memory the pixels travel in
//!   base64 inside the sequence. Reference:
//!   <https://sw.kovidgoyal.net/kitty/graphics-protocol/>.
//! - [`BlockScreen`]: colored `▀` half-block characters, two pixels per
//!   cell, for terminals without a graphics protocol. Only changed cells are
//!   rewritten.
//!
//! Drawing happens in an [`Area`] of the terminal: the whole window except
//! the last row, with the screen centered, or in inline mode only the rows
//! reserved below the prompt, with the screen at the top left like the
//! output of a command.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::Write;

use crate::vic::WIDTH;

use super::TermSize;

// ── Base64 ───────────────────────────────────────────────────────────────────

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8], out: &mut Vec<u8>) {
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { B64[(n >> (18 - 6 * i)) as usize & 63] } else { b'=' });
        }
    }
}

// ── Kitty graphics protocol ──────────────────────────────────────────────────

/// How the pixels reach the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transfer {
    /// POSIX shared memory object: only the name goes through the terminal.
    Shm,
    /// Pixels in base64 inside the sequence (e.g. over ssh).
    Direct,
}

/// Maximum scale of the pixels sent: beyond it, the terminal upscales.
/// At 3× a frame is 1209×852 RGB, about 3 MB.
const MAX_SEND_SCALE: usize = 3;

/// Assumed cell size when the terminal does not report its size in pixels.
const GUESS_CELL: (usize, usize) = (8, 16);

/// Rectangle of terminal rows holding the C64 screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Area {
    /// First row (from 0) and number of rows.
    pub row: usize,
    pub rows: usize,
    pub cols: usize,
    /// Size of a cell in pixels, if the terminal reports it.
    pub cell: Option<(usize, usize)>,
    /// Screen centered in the area; otherwise at the top left.
    pub centered: bool,
    /// Lines of the C64 screen: 284 PAL, 253 NTSC.
    pub fb_rows: usize,
}

impl Area {
    /// The whole window except the last row, which is left to the status
    /// line, for a C64 screen of `fb_rows` lines.
    pub fn fullscreen(size: TermSize, fb_rows: usize) -> Self {
        let cell = (size.xpix > 0 && size.ypix > 0).then(|| {
            ((size.xpix / size.cols.max(1)).max(1), (size.ypix / size.rows.max(1)).max(1))
        });
        Self { row: 0, rows: size.rows.saturating_sub(1).max(1), cols: size.cols.max(1), cell, centered: true, fb_rows }
    }

    /// Rows reserved below the prompt starting at `row`: those the screen
    /// needs ([`inline_rows`]), with the screen at the top left.
    pub fn inline(size: TermSize, graphics: bool, row: usize, fb_rows: usize) -> Self {
        Self { row, rows: inline_rows(size, graphics, fb_rows), centered: false, ..Self::fullscreen(size, fb_rows) }
    }
}

/// Rows for the screen in inline mode: those of the image at the largest
/// integer scale that fits the window (at most 3×, like the pixels sent),
/// not the whole window. The status line comes on top.
pub fn inline_rows(size: TermSize, graphics: bool, fb_rows: usize) -> usize {
    let full = Area::fullscreen(size, fb_rows);
    if !graphics {
        return BlockLayout::new(full).ys.len().div_ceil(2);
    }
    let (cw, ch) = full.cell.unwrap_or(GUESS_CELL);
    let fit = (full.cols * cw / WIDTH).min(full.rows * ch / fb_rows);
    if fit == 0 { return full.rows; }
    (fb_rows * fit.min(MAX_SEND_SCALE)).div_ceil(ch).min(full.rows)
}

/// Where and how the image is placed in the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placement {
    /// Top-left cell (from 0, on screen).
    pub col: usize,
    pub row: usize,
    /// Pixel offset within the cell.
    pub x_off: usize,
    pub y_off: usize,
    /// Integer factor by which the pixels are enlarged before sending.
    pub scale: usize,
    /// Cell rectangle the terminal scales the image into; `None` = native
    /// size, exact pixels.
    pub cells: Option<(usize, usize)>,
}

/// Computes the placement of the C64 screen in the area. With the pixel size
/// known and an integer scale that fits, the image goes at native size
/// (centered to the pixel if the area is centered); otherwise the terminal
/// scales it into a cell rectangle.
pub fn kitty_placement(area: Area, max_scale: usize) -> Placement {
    let (cols, rows) = (area.cols, area.rows);
    let (cw, ch) = area.cell.unwrap_or(GUESS_CELL);
    let (aw, ah) = (cols * cw, rows * ch);
    let fit = (aw / WIDTH).min(ah / area.fb_rows);

    if area.cell.is_some() && (1..=max_scale).contains(&fit) {
        let (ox, oy) = if area.centered { ((aw - WIDTH * fit) / 2, (ah - area.fb_rows * fit) / 2) } else { (0, 0) };
        return Placement {
            col: ox / cw, row: area.row + oy / ch, x_off: ox % cw, y_off: oy % ch, scale: fit, cells: None,
        };
    }
    // Integer scale if possible, otherwise the largest that fits
    let f = if fit >= 1 { fit as f64 } else { (aw as f64 / WIDTH as f64).min(ah as f64 / area.fb_rows as f64) };
    let c = ((WIDTH as f64 * f / cw as f64).round() as usize).clamp(1, cols);
    let r = ((area.fb_rows as f64 * f / ch as f64).round() as usize).clamp(1, rows);
    let (col, row) = if area.centered { ((cols - c) / 2, (rows - r) / 2) } else { (0, 0) };
    Placement {
        col,
        row: area.row + row,
        x_off: 0,
        y_off: 0,
        scale: fit.clamp(1, max_scale),
        cells: Some((c, r)),
    }
}

/// Shared memory objects used in rotation: the terminal deletes each one
/// after reading it, the leftovers are deleted on exit.
const SHM_SLOTS: u32 = 64;

pub struct KittyScreen {
    transfer: Transfer,
    /// Image id, different for each process: in inline mode the last frame
    /// stays in the terminal scrollback and a later run must not replace
    /// it.
    id: u32,
    placement: Placement,
    /// RGB pixels, already enlarged.
    rgb: Vec<u8>,
    /// Last frame sent: an identical one is not resent.
    last: Vec<u32>,
    seq: u32,
}

impl KittyScreen {
    pub fn new(transfer: Transfer, area: Area) -> Self {
        let mut s = Self {
            transfer,
            id: std::process::id().max(1),
            placement: kitty_placement(area, 1),
            rgb: Vec::new(),
            last: Vec::new(),
            seq: 0,
        };
        s.resize(area);
        s
    }

    pub fn id(&self) -> u32 { self.id }

    fn max_scale(&self) -> usize {
        // In base64 every extra pixel burdens the terminal: native size
        if self.transfer == Transfer::Shm { MAX_SEND_SCALE } else { 1 }
    }

    /// New area: the next frame is redrawn.
    pub fn resize(&mut self, area: Area) {
        self.placement = kitty_placement(area, self.max_scale());
        self.last.clear();
    }

    /// Writes to `out` the sequences that show `fb`, if it changed.
    pub fn draw(&mut self, fb: &[u32], out: &mut Vec<u8>) {
        if self.last == fb { return; }
        self.last.clear();
        self.last.extend_from_slice(fb);

        let pl = self.placement;
        let k = pl.scale;
        let lines = fb.len() / WIDTH;
        let (w, h) = (WIDTH * k, lines * k);
        self.rgb.clear();
        self.rgb.reserve(w * h * 3);
        for y in 0..lines {
            let start = self.rgb.len();
            for &px in &fb[y * WIDTH..(y + 1) * WIDTH] {
                let rgb = [(px >> 16) as u8, (px >> 8) as u8, px as u8];
                for _ in 0..k { self.rgb.extend_from_slice(&rgb); }
            }
            for _ in 1..k { self.rgb.extend_from_within(start..start + w * 3); }
        }

        let _ = write!(out, "\x1b[{};{}H", pl.row + 1, pl.col + 1);
        // Same image and same placement (p=1): they get replaced;
        // q=2 no replies, C=1 the cursor does not move
        let mut keys = format!("a=T,f=24,s={w},v={h},i={},p=1,q=2,C=1", self.id);
        if pl.x_off > 0 { keys += &format!(",X={}", pl.x_off); }
        if pl.y_off > 0 { keys += &format!(",Y={}", pl.y_off); }
        if let Some((c, r)) = pl.cells { keys += &format!(",c={c},r={r}"); }

        match self.transfer {
            Transfer::Shm => {
                let name = shm_name(self.seq);
                self.seq = (self.seq + 1) % SHM_SLOTS;
                if shm_write(&name, &self.rgb).is_err() { return; }
                let _ = write!(out, "\x1b_G{keys},t=s,S={};", self.rgb.len());
                base64(name.as_bytes(), out);
                out.extend_from_slice(b"\x1b\\");
            }
            Transfer::Direct => {
                // In chunks of 4096 base64 characters: m=1 more follow
                let mut b64 = Vec::with_capacity(self.rgb.len() * 4 / 3 + 4);
                base64(&self.rgb, &mut b64);
                let chunks: Vec<&[u8]> = b64.chunks(4096).collect();
                for (i, chunk) in chunks.iter().enumerate() {
                    let more = (i + 1 < chunks.len()) as u8;
                    if i == 0 {
                        let _ = write!(out, "\x1b_G{keys},t=d,m={more};");
                    } else {
                        let _ = write!(out, "\x1b_Gm={more};");
                    }
                    out.extend_from_slice(chunk);
                    out.extend_from_slice(b"\x1b\\");
                }
            }
        }
    }

    /// Deletes the shared memory objects not read yet.
    pub fn cleanup(&self) {
        if self.transfer == Transfer::Shm {
            for i in 0..SHM_SLOTS { shm_unlink(&shm_name(i)); }
        }
    }
}

/// Name of a shared memory object (at most 31 characters on macOS).
pub fn shm_name(slot: u32) -> String {
    format!("/c64term{}-{slot}", std::process::id())
}

/// Creates the object `name` with contents `data`. If it already exists (the
/// terminal has not read it yet) it is replaced.
pub fn shm_write(name: &str, data: &[u8]) -> std::io::Result<()> {
    let cname = CString::new(name).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let open = || unsafe {
        libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR, 0o600 as libc::c_uint)
    };
    let mut fd = open();
    if fd < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
        shm_unlink(name);
        fd = open();
    }
    if fd < 0 { return Err(std::io::Error::last_os_error()); }
    // SAFETY: valid fd just opened; the mapping covers exactly data.len()
    // bytes after ftruncate and is released before closing
    let res = unsafe {
        if libc::ftruncate(fd, data.len() as libc::off_t) != 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let p = libc::mmap(std::ptr::null_mut(), data.len(), libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
            if p == libc::MAP_FAILED {
                Err(std::io::Error::last_os_error())
            } else {
                std::ptr::copy_nonoverlapping(data.as_ptr(), p as *mut u8, data.len());
                libc::munmap(p, data.len());
                Ok(())
            }
        }
    };
    unsafe { libc::close(fd) };
    if res.is_err() { shm_unlink(name); }
    res
}

pub fn shm_unlink(name: &str) {
    if let Ok(c) = CString::new(name) {
        unsafe { libc::shm_unlink(c.as_ptr()) };
    }
}

// ── Half blocks ──────────────────────────────────────────────────────────────

/// Sampling of the C64 screen onto the half-cell grid.
struct BlockLayout {
    /// Top-left cell (from 0, on screen).
    col: usize,
    row: usize,
    /// Framebuffer column/row for each output column/half row.
    xs: Vec<usize>,
    ys: Vec<usize>,
}

impl BlockLayout {
    fn new(area: Area) -> Self {
        let (cols, rows) = (area.cols, area.rows);
        // Cells are about 1:2, so a half block is roughly square
        let (pw, ph) = (cols, rows * 2);
        let (out_w, out_h, step): (usize, usize, f64) = if pw >= WIDTH && ph >= area.fb_rows {
            let m = (pw / WIDTH).min(ph / area.fb_rows);
            (WIDTH * m, area.fb_rows * m, 1.0 / m as f64)
        } else {
            let s = (WIDTH as f64 / pw as f64).max(area.fb_rows as f64 / ph as f64);
            ((WIDTH as f64 / s) as usize, (area.fb_rows as f64 / s) as usize, s)
        };
        let xs = (0..out_w).map(|x| ((x as f64 * step) as usize).min(WIDTH - 1)).collect();
        let ys = (0..out_h).map(|y| ((y as f64 * step) as usize).min(area.fb_rows - 1)).collect();
        let out_rows = out_h.div_ceil(2);
        let (col, row) = if area.centered { ((cols - out_w) / 2, rows.saturating_sub(out_rows) / 2) } else { (0, 0) };
        Self { col, row: area.row + row, xs, ys }
    }
}

pub struct BlockScreen {
    truecolor: bool,
    layout: BlockLayout,
    /// Colors (top, bottom) already on screen; `u32::MAX` = to be drawn.
    cells: Vec<(u32, u32)>,
    ansi256: HashMap<u32, u8>,
}

impl BlockScreen {
    pub fn new(truecolor: bool, area: Area) -> Self {
        let mut s = Self { truecolor, layout: BlockLayout::new(area), cells: Vec::new(), ansi256: HashMap::new() };
        s.resize(area);
        s
    }

    /// New area (already cleared): redraw everything.
    pub fn resize(&mut self, area: Area) {
        self.layout = BlockLayout::new(area);
        let n = self.layout.xs.len() * self.layout.ys.len().div_ceil(2);
        self.cells = vec![(u32::MAX, u32::MAX); n];
    }

    /// Writes to `out` the cells changed since the last draw.
    pub fn draw(&mut self, fb: &[u32], out: &mut Vec<u8>) {
        let Self { truecolor, layout: l, cells, ansi256: palette } = self;
        let mut color = |out: &mut Vec<u8>, fg: bool, rgb: u32| {
            let layer = if fg { 38 } else { 48 };
            if *truecolor {
                let _ = write!(out, "\x1b[{layer};2;{};{};{}m", rgb >> 16, (rgb >> 8) & 0xFF, rgb & 0xFF);
            } else {
                let idx = *palette.entry(rgb).or_insert_with(|| ansi256(rgb));
                let _ = write!(out, "\x1b[{layer};5;{idx}m");
            }
        };
        let w = l.xs.len();
        let rows = l.ys.len().div_ceil(2);
        // Terminal state: cursor position and current colors
        let mut cursor: Option<(usize, usize)> = None;
        let (mut fg, mut bg) = (u32::MAX, u32::MAX);
        for r in 0..rows {
            let top = &fb[l.ys[2 * r] * WIDTH..][..WIDTH];
            let bottom = l.ys.get(2 * r + 1).map(|&y| &fb[y * WIDTH..][..WIDTH]);
            for c in 0..w {
                let t = top[l.xs[c]] & 0xFF_FFFF;
                let b = bottom.map_or(0, |row| row[l.xs[c]] & 0xFF_FFFF);
                let cell = &mut cells[r * w + c];
                if *cell == (t, b) { continue; }
                *cell = (t, b);
                let pos = (l.row + r, l.col + c);
                if cursor != Some(pos) {
                    let _ = write!(out, "\x1b[{};{}H", pos.0 + 1, pos.1 + 1);
                }
                if t == b {
                    // Single-color cell: the background is enough
                    if bg != b { color(out, false, b); bg = b; }
                    out.push(b' ');
                } else {
                    if fg != t { color(out, true, t); fg = t; }
                    if bg != b { color(out, false, b); bg = b; }
                    out.extend_from_slice("▀".as_bytes());
                }
                cursor = Some((pos.0, pos.1 + 1));
            }
        }
    }
}

/// Nearest color in the xterm 256-color palette (6×6×6 cube and grays).
fn ansi256(rgb: u32) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let (r, g, b) = ((rgb >> 16) as i32 & 0xFF, (rgb >> 8) as i32 & 0xFF, rgb as i32 & 0xFF);
    let dist = |(r2, g2, b2): (i32, i32, i32)| (r - r2).pow(2) + (g - g2).pow(2) + (b - b2).pow(2);
    let near = |v: i32| (0..6).min_by_key(|&i| (LEVELS[i] - v).abs()).unwrap();
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let cube = (16 + 36 * ri + 6 * gi + bi) as u8;
    let cube_d = dist((LEVELS[ri], LEVELS[gi], LEVELS[bi]));
    let gray_i = ((r + g + b) / 3 - 8).clamp(0, 230) / 10;
    let gv = 8 + 10 * gray_i;
    if dist((gv, gv, gv)) < cube_d { 232 + gray_i as u8 } else { cube }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vic::HEIGHT;

    #[test]
    fn base64_matches_rfc4648() {
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            let mut out = Vec::new();
            base64(input.as_bytes(), &mut out);
            assert_eq!(String::from_utf8(out).unwrap(), expected);
        }
    }

    #[test]
    fn native_placement_is_centered_to_the_pixel() {
        // 150×60 cells of 10×20 pixels: area 1500×1180, fits 3×
        let size = TermSize { cols: 150, rows: 60, xpix: 1500, ypix: 1200 };
        let p = kitty_placement(Area::fullscreen(size, HEIGHT), 3);
        assert_eq!(p.cells, None);
        assert_eq!(p.scale, 3);
        let ox = (1500 - WIDTH * 3) / 2;
        let oy = (1180 - HEIGHT * 3) / 2;
        assert_eq!((p.col * 10 + p.x_off, p.row * 20 + p.y_off), (ox, oy));
    }

    #[test]
    fn large_window_is_scaled_by_the_terminal() {
        let size = TermSize { cols: 300, rows: 100, xpix: 3000, ypix: 2000 };
        let p = kitty_placement(Area::fullscreen(size, HEIGHT), 3);
        assert_eq!(p.scale, 3);
        // Integer 6×: 2418×1704 pixels = 242×85 cells
        assert_eq!(p.cells, Some((242, 85)));
    }

    #[test]
    fn small_window_and_unknown_pixels_still_fit() {
        let p = kitty_placement(Area::fullscreen(TermSize { cols: 40, rows: 24, xpix: 320, ypix: 384 }, HEIGHT), 3);
        let (c, r) = p.cells.unwrap();
        assert!(c <= 40 && r <= 23 && p.scale == 1);
        let p = kitty_placement(Area::fullscreen(TermSize { cols: 120, rows: 40, xpix: 0, ypix: 0 }, HEIGHT), 3);
        assert!(p.cells.is_some());
    }

    #[test]
    fn block_layout_fits_the_terminal() {
        let l = BlockLayout::new(Area::fullscreen(TermSize { cols: 200, rows: 50, xpix: 0, ypix: 0 }, HEIGHT));
        assert!(l.col + l.xs.len() <= 200 && l.row + l.ys.len().div_ceil(2) <= 49);
        let l = BlockLayout::new(Area::fullscreen(TermSize { cols: 900, rows: 300, xpix: 0, ypix: 0 }, HEIGHT));
        assert_eq!(l.xs.len(), WIDTH * 2); // enlarged 2× without blurring
    }

    #[test]
    fn block_screen_redraws_only_changes() {
        let size = TermSize { cols: 403, rows: 143, xpix: 0, ypix: 0 };
        let mut s = BlockScreen::new(true, Area::fullscreen(size, HEIGHT));
        let mut fb = vec![0x000000u32; WIDTH * HEIGHT];
        let mut out = Vec::new();
        s.draw(&fb, &mut out);
        assert!(!out.is_empty());
        out.clear();
        s.draw(&fb, &mut out);
        assert!(out.is_empty());
        fb[0] = 0xFFFFFF;
        s.draw(&fb, &mut out);
        assert_eq!(out, "\x1b[1;1H\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m▀".as_bytes());
    }

    #[test]
    fn inline_area_is_only_as_tall_as_the_image() {
        // 150×60 cells of 10×20: fits 3×, 852 pixels = 43 rows
        let size = TermSize { cols: 150, rows: 60, xpix: 1500, ypix: 1200 };
        let rows = inline_rows(size, true, HEIGHT);
        assert_eq!(rows, 43);
        // In the 43-row area from row 10 the image goes native at 3×, at
        // the top left
        let p = kitty_placement(Area::inline(size, true, 10, HEIGHT), 3);
        assert_eq!((p.scale, p.cells), (3, None));
        assert_eq!((p.col, p.x_off, p.row, p.y_off), (0, 0, 10, 0));
        // Huge window: at most 3×
        assert_eq!(inline_rows(TermSize { cols: 400, rows: 200, xpix: 4000, ypix: 4000 }, true, HEIGHT), 43);
        // Half blocks: the rows of the sampled image
        assert_eq!(inline_rows(TermSize { cols: 403, rows: 300, xpix: 0, ypix: 0 }, false, HEIGHT), 142);
    }

    #[test]
    fn ansi256_picks_nearest() {
        assert_eq!(ansi256(0x000000), 16);
        assert_eq!(ansi256(0xFFFFFF), 231);
        assert_eq!(ansi256(0xFF0000), 196);
        assert_eq!(ansi256(0x808080), 244);
    }
}
